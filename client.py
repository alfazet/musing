import json
import socket
import readline

PORT = 2137
host = socket.gethostname()
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)

s.connect(("localhost", PORT))
response_len = int.from_bytes(s.recv(4), "big")
response = s.recv(response_len)
print(json.loads(response))
while True:
    msg = ""
    kind = input("kind: ").strip()
    if kind == "ls":
        dir = input("dir: ").strip()
        msg = json.dumps({"kind": "ls", "dir": dir})
    elif kind == "metadata":
        ids = list(map(int, input("ids: ").strip().split(",")))
        tags = input("tags: ").strip().split(",")
        msg = json.dumps({"kind": "metadata", "ids": ids, "tags": tags})
    elif kind == "select":
        n_filters = int(input("n_filters: ").strip())
        filters = []
        for _ in range(n_filters):
            tag = input("tag: ").strip()
            regex = input("regex: ").strip()
            filters.append({"kind": "regex", "tag": tag, "regex": regex})
        n_comparators = int(input("n_comparators: ").strip())
        comparators = []
        for _ in range(n_comparators):
            c_tag = input("tag: ").strip()
            comparators.append({"tag": c_tag})
        msg = json.dumps(
            {"kind": "select", "filters": filters, "comparators": comparators}
        )
    elif kind == "unique":
        tag = input("tag: ").strip()
        n_filters = int(input("n_filters: ").strip())
        filters = []
        for _ in range(n_filters):
            f_tag = input("tag: ").strip()
            regex = input("regex: ").strip()
            filters.append({"kind": "regex", "tag": f_tag, "regex": regex})
        n_group_by = int(input("n_group_by: ").strip())
        group_by = []
        for _ in range(n_group_by):
            g_tag = input("tag: ").strip()
            group_by.append(g_tag)
        msg = json.dumps(
            {
                "kind": "unique",
                "tag": tag,
                "filters": filters,
                "group_by": group_by,
            }
        )
    elif kind == "add":
        ids = list(map(int, input("ids: ").strip().split(",")))
        pos = int(input("pos: ").strip())
        request = {"kind": "add", "ids": ids}
        if pos >= 0:
            request["pos"] = pos
        msg = json.dumps(request)
    elif kind == "remove":
        ids = list(map(int, input("ids: ").strip().split(",")))
        msg = json.dumps({"kind": "remove", "ids": ids})
    elif kind == "play":
        id = int(input("id: ").strip())
        msg = json.dumps({"kind": "play", "id": id})
    elif kind == "setvol":
        volume = int(input("volume: ").strip())
        msg = json.dumps({"kind": "setvol", "volume": volume})
    elif kind == "changevol":
        delta = int(input("delta: ").strip())
        msg = json.dumps({"kind": "changevol", "delta": delta})
    elif kind == "seek":
        delta = int(input("seconds: ").strip())
        msg = json.dumps({"kind": "seek", "seconds": delta})
    elif kind in ("disable", "enable"):
        device = input("device: ").strip()
        msg = json.dumps({"kind": kind, "device": device})
    elif kind in (
        "previous",
        "next",
        "pause",
        "resume",
        "stop",
        "toggle",
        "gapless",
        "clear",
        "random",
        "sequential",
        "single",
        "current",
        "elapsed",
        "queue",
        "state",
        "reset",
        "update",
        "volume",
        "listdev",
    ):
        msg = json.dumps({"kind": kind})
    else:
        print("invalid request")
        continue

    msg_bytes = bytes(msg, "utf8")
    n = len(msg_bytes)
    s.sendall(n.to_bytes(4, "big"))
    s.sendall(msg_bytes)
    response_len = int.from_bytes(s.recv(4), "big")
    response = s.recv(response_len)
    print(json.loads(response))
