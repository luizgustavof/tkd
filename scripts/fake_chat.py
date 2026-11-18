"""Chat falso de live para os experimentos de caos (docs/CHAOS.md, F3). So escuta em 127.0.0.1.

    python fake_chat.py <porta> <irc|youtube|kick|kick429> <cair_depois_s>

irc      imita o IRC anonimo da Twitch (o live.py conecta com TKD_TWITCH_IRC=127.0.0.1:<porta>)
youtube  imita a pagina live_chat + youtubei get_live_chat (TKD_YOUTUBE_BASE=http://127.0.0.1:<porta>)
kick     imita /api/v2/channels/<slug> e /api/v2/channels/<id>/messages (TKD_KICK_BASE=http://127.0.0.1:<porta>)
kick429  igual ao kick, mas em vez de cair passa a responder 429 para sempre (servidor no ar, IP limitado)

Manda mensagens (com rajadas de "KEKW" de vez em quando) e, depois de <cair_depois_s>, derruba tudo e para de
escutar: as reconexoes levam "conexao recusada", como um chat fora do ar. Uma linha no stdout por evento.
"""
import http.server
import json
import os
import socket
import sys
import threading
import time

PORT, MODE, DROP = int(sys.argv[1]), sys.argv[2], float(sys.argv[3])
T0 = time.time()
DOWN = threading.Event()
sent = [0]


def say(msg):
    print(msg, flush=True)


def lines_now():
    """Mensagens desde a ultima consulta: normal 2/s, rajada a cada 20 s."""
    el = time.time() - T0
    burst = int(el) % 20 < 3
    return ["KEKW KEKW", "LUL", "no way", "KEKW"] if burst else ["gg", "nice"]


def serve_irc():
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", PORT))
    srv.listen(5)
    say(f"no ar: irc em 127.0.0.1:{PORT}")
    srv.settimeout(0.5)
    conns = []
    while not DOWN.is_set():
        try:
            c, _ = srv.accept()
            c.recv(4096)
            conns.append(c)
        except socket.timeout:
            pass
        for c in list(conns):
            try:
                for m in lines_now():
                    c.sendall(f":u!u@u.tmi.twitch.tv PRIVMSG #caos :{m}\r\n".encode())
                    sent[0] += 1
            except OSError:
                conns.remove(c)
        if time.time() - T0 > DROP:
            DOWN.set()
    for c in conns:
        try:
            c.shutdown(socket.SHUT_RDWR)
            c.close()
        except OSError:
            pass
    srv.close()
    say(f"chat caiu aos {time.time() - T0:.0f}s ({sent[0]} mensagens mandadas); servidor fora do ar")


class YT(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, body, ctype):
        b = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        self.reply('<html><script>{"INNERTUBE_API_KEY":"k","INNERTUBE_CLIENT_VERSION":"2.0",'
                   '"continuation":"c0"}</script></html>', "text/html")

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length") or 0))
        now = int(time.time() * 1e6)
        acts = []
        for i, m in enumerate(lines_now() * 4):
            acts.append({"addChatItemAction": {"item": {"liveChatTextMessageRenderer": {
                "message": {"runs": [{"text": m}]}, "timestampUsec": str(now - i * 100000)}}}})
        sent[0] += len(acts)
        self.reply(json.dumps({"continuationContents": {"liveChatContinuation": {
            "actions": acts,
            "continuations": [{"timedContinuationData": {"continuation": "c1", "timeoutMs": 8000}}]}}}),
            "application/json")


class Kick(http.server.BaseHTTPRequestHandler):
    seq = [0]

    def log_message(self, *a):
        pass

    def reply(self, code, body):
        b = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if MODE == "kick429" and time.time() - T0 > DROP:
            if not DOWN.is_set():
                DOWN.set()
                say(f"chat caiu aos {time.time() - T0:.0f}s ({sent[0]} mensagens mandadas): 429 daqui em diante")
            self.reply(429, '{"message":"Too Many Requests"}')
            return
        if not self.path.endswith("/messages"):
            self.reply(200, '{"id": 4242, "slug": "caos"}')
            return
        msgs = []
        for m in lines_now():
            self.seq[0] += 1
            sent[0] += 1
            msgs.append({"id": f"m{self.seq[0]}", "content": m,
                         "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())})
        self.reply(200, json.dumps({"data": {"messages": msgs[::-1]}}))


def serve_kick():
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Kick)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    say(f"no ar: kick em http://127.0.0.1:{PORT} (modo {MODE})")
    if MODE == "kick429":
        return
    while time.time() - T0 < DROP:
        time.sleep(0.5)
    srv.shutdown()
    srv.server_close()
    say(f"chat caiu aos {time.time() - T0:.0f}s ({sent[0]} mensagens mandadas); servidor fora do ar")


def serve_youtube():
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), YT)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    say(f"no ar: youtube em http://127.0.0.1:{PORT}")
    while time.time() - T0 < DROP:
        time.sleep(0.5)
    srv.shutdown()
    srv.server_close()
    say(f"chat caiu aos {time.time() - T0:.0f}s ({sent[0]} mensagens mandadas); servidor fora do ar")


{"irc": serve_irc, "youtube": serve_youtube, "kick": serve_kick, "kick429": serve_kick}[MODE]()
time.sleep(3600)
os._exit(0)
