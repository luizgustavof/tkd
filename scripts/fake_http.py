"""Servidor HTTP local para os experimentos de caos de rede (docs/CHAOS.md, secao C). So escuta em 127.0.0.1.

    python fake_http.py <porta> <arquivo> <modo>

Serve <arquivo> em http://127.0.0.1:<porta>/v.mp4. Modos:
  drop   manda ~40% do arquivo devagar e derruba a conexao; depois disso o servidor fecha de vez (internet caiu:
         as novas tentativas do downloader levam "conexao recusada")
  stall  manda ~40% e para de mandar, com a conexao aberta (rede presa sem fechar)
  ok     serve normal (controle)

Escreve no stdout o que fez, uma linha por evento (a bancada guarda no relatorio).
"""
import http.server
import os
import socket
import sys
import threading
import time

PORT, SRC, MODE = int(sys.argv[1]), sys.argv[2], sys.argv[3]
DATA = open(SRC, "rb").read()
DONE = threading.Event()


def say(msg):
    print(msg, flush=True)


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_HEAD(self):
        self.send_response(200)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Content-Length", str(len(DATA)))
        self.send_header("Accept-Ranges", "none")
        self.end_headers()

    def do_GET(self):
        if self.path.split("?")[0] != "/v.mp4":
            self.send_error(404)
            return
        self.do_HEAD()
        if MODE == "ok":
            self.wfile.write(DATA)
            return
        cut = int(len(DATA) * 0.4)
        sent = 0
        try:
            while sent < cut:
                n = min(64 * 1024, cut - sent)
                self.wfile.write(DATA[sent:sent + n])
                self.wfile.flush()
                sent += n
                time.sleep(0.05)
        except OSError:
            return
        if MODE == "stall":
            say(f"travou a conexao aos {sent} de {len(DATA)} bytes")
            DONE.wait(3600)
            return
        say(f"derrubou a conexao aos {sent} de {len(DATA)} bytes; servidor fora do ar")
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.connection.close()
        DONE.set()


srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H)
srv.daemon_threads = True
threading.Thread(target=srv.serve_forever, daemon=True).start()
say(f"no ar em http://127.0.0.1:{PORT}/v.mp4 ({len(DATA)} bytes, modo {MODE})")
try:
    if MODE == "drop":
        DONE.wait(3600)
        time.sleep(0.2)
        srv.shutdown()
        srv.server_close()
        time.sleep(3600)
    else:
        time.sleep(3600)
except KeyboardInterrupt:
    pass
os._exit(0)
