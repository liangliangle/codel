#!/usr/bin/env python3
"""Remove the telemetry-donation chain that the fork dropped.

The upstream tree donates logs/metrics/traces to the hub (`donate_pump`,
`log_donate`, `metric_donate`, `trace_donate`).  codel removed it: the SDK's
manifest has none of its deps, `lib.rs` declared no such modules, and no call
site existed.  Re-apply that removal after an upstream sync.

Idempotent: prints what it changed, does nothing if already excised.
"""
import os
import re
import sys

ROOT = '/Users/zhangqiong/Desktop/Project/codel'


def edit(rel, subs, required=False):
    p = os.path.join(ROOT, rel)
    if not os.path.exists(p):
        print(f"  !! missing {rel}")
        return 0
    s = open(p).read()
    n = 0
    for a, b in subs:
        if a in s:
            s = s.replace(a, b)
            n += 1
        elif required:
            print(f"  !! pattern not found in {rel}: {a[:60]!r}")
    if n:
        open(p, 'w').write(s)
        print(f"  {rel}: {n} edit(s)")
    return n


def drop_fns(rel, names):
    """Delete `fn <name>(...) { ... }` plus an immediately preceding doc comment."""
    p = os.path.join(ROOT, rel)
    s = open(p).read()
    removed = []
    for name in names:
        m = re.search(r'\n((?:\s*///[^\n]*\n)*)(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn ' + re.escape(name) + r'\b', s)
        if not m:
            continue
        start = m.start() + 1
        b = s.index('{', m.end())
        depth = 0
        k = b
        while k < len(s):
            if s[k] == '{':
                depth += 1
            elif s[k] == '}':
                depth -= 1
                if depth == 0:
                    break
            k += 1
        s = s[:start] + s[k + 1:]
        removed.append(name)
    if removed:
        open(p, 'w').write(s)
        print(f"  {rel}: dropped fns {removed}")


def main():
    # 1. SDK: no donation modules.
    edit('crates/common/codel-computer-hub-sdk/src/lib.rs', [
        ('pub(crate) mod donate_pump;\n', ''),
        ('pub mod log_donate;\n', ''),
        ('#[cfg(feature = "metrics")]\npub mod metric_donate;\n', ''),
        ('pub mod trace_donate;\n', ''),
        ('pub use log_donate::{DonatingLogLayer, LogDonationPump, LogDonationSender, flush_log_layer};\n', ''),
        ('#[cfg(feature = "metrics")]\npub use metric_donate::MetricDonationPump;\n', ''),
        ('pub use trace_donate::{HubDonatingReporter, TraceDonationPump};\n', ''),
    ])
    for f in ['donate_pump.rs', 'log_donate.rs', 'metric_donate.rs', 'trace_donate.rs']:
        fp = os.path.join(ROOT, 'crates/common/codel-computer-hub-sdk/src', f)
        if os.path.exists(fp):
            os.remove(fp)
            print(f"  removed {f}")
    # 2. SDK: no span attachment into the hub socket.
    edit('crates/common/codel-computer-hub-sdk/src/connection.rs', [
        ('    codel_tracing::http_client::attach_trace_to_http_request(headers);\n', ''),
    ])
    # 3. Workspace handle: no donation entry points.
    drop_fns('crates/codegen/codel-workspace/src/handle.rs',
             ['trace_donation_reporter', 'log_donation_layer', 'metric_donation_reporter'])
    # 4. Workspace tests: drop the inert-entry-point test.
    p = os.path.join(ROOT, 'crates/codegen/codel-workspace/src/handle_tests.rs')
    if os.path.exists(p):
        s = open(p).read()
        i = s.find('/// Without a connection every export entry point returns `None`')
        j = s.find('#[test]\nfn rewind_outcome_label_maps_each_variant')
        if i >= 0 and j > i:
            open(p, 'w').write(s[:i] + s[j:])
            print("  handle_tests.rs: dropped donation test")
    # 5. workspace-server binary: no direct OTLP, no donation pumps.
    edit('crates/codegen/codel-workspace/src/bin/workspace_server.rs', [
        ('''    let direct_otlp = match std::env::var("CODEL_WORKSPACE_OTLP_ENDPOINT") {
        Ok(endpoint) if !endpoint.is_empty() => {
            match codel_tracing::init_fastrace(endpoint.clone(), SERVICE_NAME.to_owned(), None) {
                Ok(()) => {
                    tracing::info!(%endpoint, "trace export enabled (direct OTLP)");
                    true
                }
                Err(e) => {
                    tracing::warn!(error = %e, "direct OTLP trace export init failed");
                    false
                }
            }
        }
        _ => false,
    };
''', ''),
        ('''    let mut donation_pump = None;
    if !direct_otlp {
        match ws_handle.trace_donation_reporter(SERVICE_NAME).await {
            Some((reporter, pump)) => {
                fastrace::set_reporter(reporter, fastrace::collector::Config::default());
                donation_pump = Some(pump);
                tracing::info!("trace export enabled");
            }
            None => tracing::info!("trace export disabled (not connected)"),
        }
    }
    let mut log_donation_pump = None;
    match ws_handle.log_donation_layer(SERVICE_NAME).await {
        Some((sender, pump)) => {
            donating.activate(sender);
            log_donation_pump = Some(pump);
            tracing::info!("log export enabled");
        }
        None => tracing::info!("log export disabled (not connected)"),
    }
    let mut metric_donation_pump = None;
    match ws_handle.metric_donation_reporter(SERVICE_NAME).await {
        Some(pump) => {
            metric_donation_pump = Some(pump);
            tracing::info!("metric export enabled");
        }
        None => tracing::info!("metric export disabled (not connected)"),
    }
    if metric_donation_pump.is_some()
        && let Some((tx, control_port)) = &preview_shutdown
    {
        tokio::spawn(preview_supervisor::supervise_preview_metrics(
            *control_port,
            tx.subscribe(),
        ));
    }
''', ''),
        ('''/// OTLP `service.name` for this binary's exported traces/logs/metrics and direct-OTLP fastrace export.
/// Single source so the call sites can't drift.
const SERVICE_NAME: &str = "prod_codel_workspace";
''', ''),
    ])
    # 6. preview-supervisor: drop the metrics scraper + its tests.
    p = os.path.join(ROOT, 'crates/codegen/codel-workspace-daemon/src/preview_supervisor.rs')
    if os.path.exists(p):
        s = open(p).read()
        i = s.find('// ── Preview-metrics scraper')
        j = s.find('#[cfg(test)]')
        if i >= 0 and j > i:
            s = s[:i] + s[j:]
            open(p, 'w').write(s)
            print("  preview_supervisor.rs: dropped metrics scraper section")
        drop_fns('crates/codegen/codel-workspace-daemon/src/preview_supervisor.rs',
                 ['metrics_url_targets_the_loopback_control_metrics_path',
                  'metrics_scrape_loop_donates_the_body_immediately_and_stops_on_shutdown',
                  'metrics_scrape_loop_skips_error_responses_and_absent_proxy',
                  'metrics_scrape_loop_returns_immediately_when_already_shut_down'])
    # 7. leader server: drop metric-donation plumbing.
    edit('crates/codegen/codel-shell/src/leader/server.rs', [
        ('''    /// Drained before the hub connection closes and re-armed on resume, since the pump is
    /// bound to one hub connection. `None` when the connect left no hub handle.
    metric_donation: Mutex<Option<codel_computer_hub_sdk::MetricDonationPump>>,
''', ''),
        ('''/// Service name the hub allowlists for the leader's metric donation.
const LEADER_METRIC_SERVICE: &str = "codel_leader";
''', ''),
        ('''    let metric_donation = exposure.metric_donation.lock().take();
    if let Some(pump) = metric_donation {
        drain_metric_donation(pump).await;
    }
''', ''),
        ('    let metric_donation = arm_metric_donation(&handle).await;\n', ''),
        ('        metric_donation: Mutex::new(metric_donation),\n', ''),
        ('''        let metric_donation = arm_metric_donation(&exp.handle).await;
        *exp.metric_donation.lock() = metric_donation;
''', ''),
    ])
    drop_fns('crates/codegen/codel-shell/src/leader/server.rs',
             ['arm_metric_donation', 'drain_metric_donation'])
    # 8. protocol: drop the donation wire types, methods and re-exports.
    p = os.path.join(ROOT, 'crates/common/codel-tool-protocol/src/methods.rs')
    if os.path.exists(p):
        s = open(p).read()
        for blk in ['''    /// Notification (no `id`, no response); rejects surface only in
    /// hub metrics. Only hub-minted trace-ids are accepted.
    TracesDonate => "traces.donate",
''', '''    /// Notification (no `id`, no response); rejects surface only in hub
    /// metrics. Donor service.name must be hub-allowlisted.
    LogsDonate => "logs.donate",
''', '''    /// Notification (no `id`, no response); rejects surface only in hub
    /// metrics. Donor service.name must be hub-allowlisted. No envelope
    /// `session_id` — metrics are process-aggregate.
    MetricsDonate => "metrics.donate",
''']:
            if blk in s:
                s = s.replace(blk, '')
        open(p, 'w').write(s)
        print("  methods.rs: donation methods removed")
    p = os.path.join(ROOT, 'crates/common/codel-tool-protocol/src/lib.rs')
    if os.path.exists(p):
        s = open(p).read()
        for name in ['LogsDonateParams, ', 'MAX_DONATION_BYTES, ', 'MAX_LOG_RECORDS_PER_DONATION, ',
                     'MAX_METRICS_PER_DONATION, ', 'MAX_SPANS_PER_DONATION, ', 'MetricsDonateParams,\n    ',
                     'TracesDonateParams, ']:
            s = s.replace(name, '')
        open(p, 'w').write(s)
        print("  tool-protocol lib.rs: donation re-exports removed")
    # frames.rs: drop the donation definitions
    p = os.path.join(ROOT, 'crates/common/codel-tool-protocol/src/frames.rs')
    if os.path.exists(p):
        s = open(p).read()
        for marker in ['// ── Trace donation', '// ── Log donation', '// ── Metric donation']:
            i = s.find(marker)
            if i < 0:
                continue
            # cut until the next top-level item after the struct's closing brace
            m = re.search(r'\n\}\n', s[i:])
            end = i + m.end() if m else len(s)
            s = s[:i] + s[end:]
        i = s.find('    // ── Donation params ──')
        if i >= 0:
            j = s.find('    // ── ToolServerStatusPayload', i)
            if j > i:
                s = s[:i] + s[j:]
        open(p, 'w').write(s)
        print("  frames.rs: donation types removed")


if __name__ == '__main__':
    main()
