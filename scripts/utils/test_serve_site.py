"""Tests of the preview server, runnable without a test framework.

    python3 -m unittest discover -s scripts/utils -p 'test_*.py'
"""

import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path

import serve_site

REPOSITORY = Path(__file__).resolve().parents[2]


class ResolveTest(unittest.TestCase):
    def setUp(self):
        self.roots = serve_site.default_roots(REPOSITORY)

    def test_the_root_answers_with_the_page(self):
        self.assertEqual(serve_site.resolve(self.roots, "/"), REPOSITORY / "site/index.html")

    def test_the_mark_comes_from_the_icon_directory(self):
        self.assertEqual(
            serve_site.resolve(self.roots, "/assets/icon.svg"),
            REPOSITORY / "assets/icon/icon.svg",
        )

    def test_the_favicon_comes_from_the_icon_directory(self):
        self.assertEqual(
            serve_site.resolve(self.roots, "/assets/favicon-32.png"),
            REPOSITORY / "assets/icon/favicon-32.png",
        )

    def test_a_screenshot_still_comes_from_the_page(self):
        self.assertEqual(
            serve_site.resolve(self.roots, "/assets/target.jpg"),
            REPOSITORY / "site/assets/target.jpg",
        )

    def test_a_file_no_mount_holds_is_not_found(self):
        self.assertIsNone(serve_site.resolve(self.roots, "/assets/absent.svg"))

    def test_a_parent_directory_never_escapes_a_mount(self):
        self.assertIsNone(serve_site.resolve(self.roots, "/../../etc/passwd"))


class ServerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = serve_site.make_server(serve_site.default_roots(REPOSITORY), 0)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def fetch(self, path):
        with urllib.request.urlopen(self.base + path) as response:
            return response.status, response.headers["Content-Type"], response.read()

    def test_the_page_and_both_icons_are_served(self):
        status, content_type, body = self.fetch("/")
        self.assertEqual(status, 200)
        self.assertEqual(content_type, "text/html")
        self.assertIn(b"assets/icon.svg", body)

        status, content_type, _ = self.fetch("/assets/icon.svg")
        self.assertEqual((status, content_type), (200, "image/svg+xml"))

        status, content_type, _ = self.fetch("/assets/favicon-32.png")
        self.assertEqual((status, content_type), (200, "image/png"))

    def test_a_file_no_mount_holds_answers_404(self):
        with self.assertRaises(urllib.error.HTTPError) as raised:
            self.fetch("/assets/absent.svg")
        self.assertEqual(raised.exception.code, 404)
        raised.exception.close()


if __name__ == "__main__":
    unittest.main()
