import sys, re

RULES = [
    # the fork folded the telemetry crate into a local logging crate
    ("xai-grok-telemetry", "codel-logging"),
    ("xai_grok_telemetry", "codel_logging"),
    ("codel-telemetry", "codel-logging"),
    ("codel_telemetry", "codel_logging"),
    # most specific domains first
    ("cli-chat-proxy.grok.com", "cli-chat-proxy.codel.dev"),
    ("computer-hub.grok.com", "computer-hub.codel.dev"),
    ("proxy.grok.com", "proxy.codel.dev"),
    ("api.x.ai", "api.codel.dev"),
    ("auth.x.ai", "auth.codel.dev"),
    ("console.x.ai", "console.codel.dev"),
    ("grok.com", "codel.dev"),
    ("x.ai/", "codel/"),
    ("x.ai", "codel.dev"),
    # xai identifiers
    ("xai-grok-", "codel-"),
    ("xai_grok_", "codel_"),
    ("XaiGrok", "Codel"),
    ("XAI_GROK", "CODEL"),
    ("xai-grok", "codel"),
    ("xai_grok", "codel"),
    ("xai-", "codel-"),
    ("xai_", "codel_"),
    ("xai", "codel"),
    ("Xai", "Codel"),
    ("XAI", "CODEL"),
    ("xAI", "Codel"),
    # grok
    ("grok", "codel"),
    ("Grok", "Codel"),
    ("GROK", "CODEL"),
]


def xform(text: str) -> str:
    for a, b in RULES:
        text = text.replace(a, b)
    return text


if __name__ == "__main__":
    sys.stdout.write(xform(sys.stdin.read()))
