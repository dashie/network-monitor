#!/usr/bin/env python3
"""Serve the landing page for local preview, icons included.

# What it does

`site/` is the page, but it is not the whole site. The mark and the favicon live once, in
`assets/icon/`, because the same two files are embedded in the executable by
`src/web/mod.rs`; the Pages workflow copies them into `_site/assets/` when it assembles
what it publishes ([.github/workflows/pages.yml](../../.github/workflows/pages.yml)).

Serving `site/` alone therefore answers `404` for `/assets/icon.svg` and
`/assets/favicon-32.png`, and the page shows an empty image where the mark should be.

This server performs that copy in memory: it answers a request from the first directory
whose file exists, mounted at the address the page asks for. The URLs are the ones GitHub
Pages serves, no `_site/` is written, and an edit to the page is visible on the next
reload.

    python3 scripts/utils/serve_site.py                 # http://127.0.0.1:8010/
    python3 scripts/utils/serve_site.py --port 9000

# The mounts, in order

    /         -> site/           the page, its stylesheet, the four JPEGs, the crawl files
    /assets/  -> assets/icon/    the mark and the favicon the workflow copies here

The first mount that holds the file answers, so `/assets/target.jpg` still comes from the
page and `/assets/icon.svg` falls through to the icon directory. Standard library only, one
file, no build step — like `convert_heartbeats.py` next to it.
"""

import argparse
import posixpath
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

DEFAULT_PORT = 8010

MOUNTS = (
    ("/", "site"),
    ("/assets/", "assets/icon"),
)


def default_roots(base):
    """The mounts of this repository, resolved against `base`."""
    return tuple((prefix, (Path(base) / directory).resolve()) for prefix, directory in MOUNTS)


def resolve(roots, url_path):
    """The file `url_path` names, or None when no mount holds it.

    `roots` is a sequence of `(prefix, directory)` pairs, searched in order.
    """
    path = unquote(urlsplit(url_path).path) or "/"
    for prefix, directory in roots:
        if not path.startswith(prefix):
            continue
        relative = posixpath.normpath(path[len(prefix):] or ".")
        if relative == ".":
            relative = "index.html"
        candidate = (directory / relative).resolve()
        if directory not in candidate.parents:
            continue
        if candidate.is_file():
            return candidate
    return None


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, roots, **kwargs):
        self.roots = roots
        super().__init__(*args, **kwargs)

    def translate_path(self, path):
        found = resolve(self.roots, path)
        if found is None:
            return str(Path(self.roots[0][1]) / "not-found")
        return str(found)


def make_server(roots, port):
    return ThreadingHTTPServer(("127.0.0.1", port), partial(Handler, roots=roots))


def main():
    parser = argparse.ArgumentParser(description="Serve site/ with the icons of assets/icon/.")
    parser.add_argument(
        "--port",
        type=int,
        default=DEFAULT_PORT,
        help=f"port to bind (default {DEFAULT_PORT})",
    )
    options = parser.parse_args()

    roots = default_roots(Path(__file__).resolve().parents[2])
    server = make_server(roots, options.port)
    for prefix, directory in roots:
        print(f"{prefix:<10} -> {directory}")
    print(f"listening on http://127.0.0.1:{server.server_address[1]}/")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
