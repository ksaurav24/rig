#!/usr/bin/env python3
"""A stand-in for GitHub, for scripts/e2e.sh.

  github-mock.py <api-port> <git-port> <repos-dir> <cert> <key> <app-key-pem>

* HTTP on <api-port> answers the few REST calls Rig makes: the manifest
  conversion, installation lookup, installation tokens and repository list.
  Calls authenticated as the App must carry a JWT (three dot-separated parts).
* HTTPS on <git-port> serves bare repositories under <repos-dir> over Git's
  smart HTTP protocol (git http-backend), but only to requests that send the installation token
  as basic auth, so a clone without it fails like a private repository would.

Every request is appended to <repos-dir>/requests.log (method, path and
whether it was authorized) so the e2e script can assert on them.
"""

import base64
import json
import os
import ssl
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

API_PORT, GIT_PORT = int(sys.argv[1]), int(sys.argv[2])
REPOS, CERT, KEY, APP_KEY = sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6]
TOKEN = "ghs_mocktoken0123456789"
INSTALLED = {"acme/site"}
LOG = os.path.join(REPOS, "requests.log")
lock = threading.Lock()


def log(line):
    with lock, open(LOG, "a") as f:
        f.write(line + "\n")


def is_jwt(header):
    return header.startswith("Bearer ") and header[7:].count(".") == 2


class Api(BaseHTTPRequestHandler):
    def reply(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass

    def handle_any(self):
        auth = self.headers.get("authorization", "")
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n) if n else b""
        log(f"api {self.command} {self.path} jwt={is_jwt(auth)} body={body.decode(errors='replace')}")
        p = self.path.split("?")[0]
        if self.command == "POST" and p.startswith("/app-manifests/") and p.endswith("/conversions"):
            with open(APP_KEY) as f:
                pem = f.read()
            return self.reply(201, {
                "id": 4242, "slug": "rig-e2e", "name": "Rig e2e",
                "html_url": f"https://127.0.0.1:{GIT_PORT}/apps/rig-e2e",
                "owner": {"login": "acme"}, "client_id": "Iv1.e2e",
                "client_secret": "mock-client-secret", "webhook_secret": "mock-webhook-secret",
                "pem": pem,
            })
        if not is_jwt(auth) and not auth == f"Bearer {TOKEN}":
            return self.reply(401, {"message": "Bad credentials"})
        if self.command == "GET" and p.startswith("/repos/") and p.endswith("/installation"):
            repo = p[len("/repos/"):-len("/installation")].lower()
            return self.reply(200, {"id": 7}) if repo in INSTALLED else self.reply(404, {"message": "Not Found"})
        if self.command == "GET" and p == "/app/installations":
            return self.reply(200, [{"id": 7}])
        if self.command == "POST" and p == "/app/installations/7/access_tokens":
            return self.reply(201, {"token": TOKEN})
        if self.command == "GET" and p == "/installation/repositories":
            return self.reply(200, {"repositories": [{
                "full_name": "acme/site", "clone_url": f"https://127.0.0.1:{GIT_PORT}/acme/site.git",
                "default_branch": "main", "private": True,
            }]})
        self.reply(404, {"message": "Not Found"})

    do_GET = do_POST = handle_any


class Git(BaseHTTPRequestHandler):
    """Smart HTTP through `git http-backend`, so shallow clones work."""

    def log_message(self, *args):
        pass

    def handle_git(self):
        want = base64.b64encode(f"x-access-token:{TOKEN}".encode()).decode()
        scheme, _, cred = self.headers.get("authorization", "").partition(" ")
        ok = scheme.lower() == "basic" and cred == want
        log(f"git {self.command} {self.path} authorized={ok}")
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n) if n else b""
        if not ok:
            self.send_response(401)
            self.send_header("www-authenticate", 'Basic realm="mock"')
            self.send_header("content-length", "0")
            self.end_headers()
            return
        path, _, query = self.path.partition("?")
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "GIT_PROJECT_ROOT": REPOS, "GIT_HTTP_EXPORT_ALL": "1",
            "PATH_INFO": path, "QUERY_STRING": query, "REQUEST_METHOD": self.command,
            "CONTENT_TYPE": self.headers.get("content-type", ""), "CONTENT_LENGTH": str(len(body)),
            "GIT_PROTOCOL": self.headers.get("git-protocol", ""), "REMOTE_ADDR": "127.0.0.1",
        }
        if self.headers.get("content-encoding") == "gzip":
            env["HTTP_CONTENT_ENCODING"] = "gzip"
        out = subprocess.run(["git", "http-backend"], input=body, env=env, capture_output=True).stdout
        head, _, payload = out.partition(b"\r\n\r\n")
        code = 200
        headers = []
        for line in head.decode(errors="replace").split("\r\n"):
            k, _, v = line.partition(":")
            if k.lower() == "status":
                code = int(v.strip().split()[0])
            elif k:
                headers.append((k, v.strip()))
        self.send_response(code)
        for k, v in headers:
            self.send_header(k, v)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = do_POST = handle_git


api = ThreadingHTTPServer(("127.0.0.1", API_PORT), Api)
git = ThreadingHTTPServer(("127.0.0.1", GIT_PORT), Git)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(CERT, KEY)
git.socket = ctx.wrap_socket(git.socket, server_side=True)
threading.Thread(target=git.serve_forever, daemon=True).start()
api.serve_forever()
