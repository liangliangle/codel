import sys, os

def mappath(p: str) -> str:
    """Map a grok-build repo path to the corresponding codel repo path."""
    p = p.replace('xai-grok-', 'codel-').replace('xai-', 'codel-')
    p = p.replace('grok_build', 'codel_build').replace('GrokBuild', 'CodelBuild')
    p = p.replace('grok_build_concise', 'codel_build_concise')
    p = p.replace('grok_build_hashline', 'codel_build_hashline')
    p = p.replace('grok', 'codel').replace('Grok', 'Codel')
    return p
