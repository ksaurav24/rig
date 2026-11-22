"""Tiny HTTP sink for the e2e alert checks: stores each POST as <dir>/<n>.body and <n>.sig."""
import http.server
import pathlib
import sys

out = pathlib.Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)


class Sink(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        n = len(list(out.glob("*.body")))
        (out / f"{n}.sig").write_text(self.headers.get("x-rig-signature", ""))
        (out / f"{n}.body").write_bytes(body)
        self.send_response(500 if b"fail-me" in self.path.encode() else 204)
        self.end_headers()

    def log_message(self, *_):
        pass


http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Sink).serve_forever()
