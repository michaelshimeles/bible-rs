"""A stand-in for GitHub's API and release downloads (PR #66 evidence, scratch branch only).

Serves <root> on 127.0.0.1:<port>, sends .exe files at about 640 KB/s so the progress bar shows,
and appends one line per request to <log>.
"""
import functools
import http.server
import sys
import time

root, port, log = sys.argv[1], int(sys.argv[2]), sys.argv[3]


class Handler(http.server.SimpleHTTPRequestHandler):
    def copyfile(self, source, dest):
        if not self.path.endswith(".exe"):
            return super().copyfile(source, dest)
        while chunk := source.read(64 * 1024):
            dest.write(chunk)
            time.sleep(0.1)

    def log_request(self, code="-", size="-"):
        with open(log, "a") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {self.command} {self.path} {code}\n")

    def log_message(self, *args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(Handler, directory=root)).serve_forever()
