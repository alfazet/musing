import socket
import readline

PORT = 2137
BUF_SIZE = 2**20
host = socket.gethostname()
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)

s.connect(("localhost", PORT))
print(s.recv(BUF_SIZE))
while True:
    msg = input("> ").strip()
    if msg == "end":
        s.shutdown(socket.SHUT_WR)
        break
    else:
        msg_bytes = bytes(msg, "utf8")
        n = len(msg_bytes)
        s.sendall(n.to_bytes(4, "big"))
        s.sendall(msg_bytes)
        print(s.recv(BUF_SIZE))
