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
        paths = input("paths: ").strip().split(";")
        tags = input("tags: ").strip().split(";")
        msg = json.dumps({"kind": "metadata", "paths": paths, "tags": tags})
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
    elif kind == "addqueue":
        paths = input("paths: ").strip().split(";")
        pos = int(input("pos: ").strip())
        request = {"kind": "addqueue", "paths": paths}
        if pos >= 0:
            request["pos"] = pos
        msg = json.dumps(request)
    elif kind == "removequeue":
        ids = list(map(int, input("ids: ").strip().split(";")))
        msg = json.dumps({"kind": "removequeue", "ids": ids})
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
    elif kind == "speed":
        speed = int(input("speed: ").strip())
        msg = json.dumps({"kind": "speed", "speed": speed})
    elif kind in ("disable", "enable"):
        device = input("device: ").strip()
        msg = json.dumps({"kind": kind, "device": device})
    elif kind == "fromfile":
        path = input("path: ").strip()
        msg = json.dumps({"kind": "fromfile", "path": path})
    elif kind == "save":
        path = input("path: ").strip()
        msg = json.dumps({"kind": "save", "path": path})
    elif kind == "listsongs":
        playlist = input("playlist: ").strip()
        msg = json.dumps({"kind": "listsongs", "playlist": playlist})
    elif kind == "removeplaylist":
        playlist = input("playlist: ").strip()
        pos = int(input("pos: ").strip())
        msg = json.dumps({"kind": "removeplaylist", "playlist": playlist, "pos": pos})
    elif kind == "load":
        playlist = input("playlist: ").strip()
        range_l = int(input("range_l: ").strip())
        range_r = int(input("range_r: ").strip())
        pos = int(input("pos: ").strip())
        request = {"kind": "load", "playlist": playlist}
        if range_l >= 0 and range_r >= 0:
            request["range"] = [range_l, range_r]
        if pos >= 0:
            request["pos"] = pos
        msg = json.dumps(request)
    elif kind == "addplaylist":
        playlist = input("playlist: ").strip()
        song = input("song: ").strip()
        msg = json.dumps({"kind": "addplaylist", "playlist": playlist, "song": song})
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
        "playlists",
        "queue",
        "state",
        "update",
        "devices",
    ):
        msg = json.dumps({"kind": kind})
    else:
        print("invalid request")
        continue

    msg_bytes = bytes(msg, "utf8")
    n = len(msg_bytes)
    s.sendall(n.to_bytes(4, "big"))
    s.sendall(msg_bytes)
    expected_len = int.from_bytes(s.recv(4), "big")
    response = bytearray()
    while len(response) < expected_len:
        response.extend(s.recv(expected_len))
    print(json.loads(response))
