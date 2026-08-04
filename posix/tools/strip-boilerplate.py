#!/usr/bin/env python3
"""Strip Open Group page furniture from a POSIX spec HTML page.

Runs before pandoc. Only handles the things that are easier to fix in HTML
than in the pandoc AST: navigation blocks, publisher banners, and the two
places where the source encodes structure in something pandoc discards
(the built-in delimiter comments, and the SYNOPSIS blockquote class).

Reads a file, writes the cleaned HTML to stdout.
"""

import re
import sys

# The built-in utilities in XCU 2.15 have no heading of their own. Each is
# introduced by a pair of anchors and an HTML comment naming it, immediately
# before its NAME section. Promote that to a real heading carrying the
# section's tag_ anchor, or the man-page sections below it are orphaned.
BUILTIN_DELIMITER = re.compile(
    r'<a\s+name="(?P<slug>[a-z][a-z0-9_.:-]*)"[^>]*>\s*</a>\s*'
    r'<a\s+name=\s*"(?P<tag>tag_[0-9_]+)"[^>]*>\s*</a>\s*'
    r'<!--\s*(?P=slug)\s*-->',
    re.S,
)

# <blockquote class="synopsis"><p><code>…</code></p></blockquote> is a command
# synopsis, not a quotation. pandoc's BlockQuote carries no attributes, so the
# class would be lost; rewrite to <pre> to land as a code block.
SYNOPSIS = re.compile(
    r'<blockquote class="synopsis">\s*<p>(?P<body>.*?)</p>\s*</blockquote>',
    re.S,
)


def strip(html: str) -> str:
    # Navigation tables above and below every built-in. No nested <div>, so a
    # non-greedy match to the first </div> is exact.
    html = re.sub(r'<div class="NAVHEADER">.*?</div>', '', html, flags=re.S)

    # The option-code popup helper, and font-size hacks sprinkled mid-content.
    html = re.sub(r'<script\b.*?</script>', '', html, flags=re.S)
    html = re.sub(r'<basefont\b[^>]*>', '', html)

    # Publisher banners only. Other <center> blocks wrap normative tables
    # (parameter expansion, shell errors, pipefail) and must survive.
    html = re.sub(r'<center><font size="2">.*?</center>', '', html, flags=re.S)

    html = re.sub(
        r'<a href="#top"><span class="topOfPage">.*?</span></a>\s*(?:<br>)?',
        '', html, flags=re.S,
    )
    html = re.sub(r'<p>&nbsp;</p>', '', html)
    html = re.sub(r'<hr\b[^>]*>', '', html)

    html = BUILTIN_DELIMITER.sub(
        r'<h3 class="builtin" id="\g<tag>">\g<slug></h3>', html,
    )
    html = SYNOPSIS.sub(r'<pre class="synopsis">\g<body></pre>', html)

    return html


def main() -> int:
    if len(sys.argv) != 2:
        print(f'usage: {sys.argv[0]} <page.html>', file=sys.stderr)
        return 2
    with open(sys.argv[1], encoding='utf-8') as fh:
        sys.stdout.write(strip(fh.read()))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
