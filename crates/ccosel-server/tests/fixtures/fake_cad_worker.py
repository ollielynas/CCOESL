"""A stand-in for cad/worker.py that needs no FreeCAD: same protocol, canned geometry.

It lets the server's side of CAD — starting the worker, matching answers to requests,
restarting it when it dies, jobs, the mesh store, export — be tested anywhere Python is.
`volume` in each rebuild's answer counts the rebuilds this worker has done, so a test can
tell a cached job from a fresh one. A push/pull of face 999 fails as FreeCAD would; of face
666 the worker dies mid-request.
"""

import json
import sys

rebuilds = 0


def answer(msg):
    sys.stdout.write("@@CCOSEL " + json.dumps(msg) + "\n")
    sys.stdout.flush()


# FreeCAD prints a banner; the server must ignore anything that is not an answer.
print("FreeCAD (fake) starting")
answer({"ready": True, "version": "fake"})

SQUARE = {
    "positions": [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]],
    "triangles": [[0, 1, 2], [0, 2, 3]],
    "tri_face": [0, 0],
    "face_normals": [[0, 0, 1]],
    "loose": [True],
    "edges": [[[0, 0, 0], [10, 0, 0]], [[10, 0, 0], [10, 10, 0]]],
    "vertices": [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]],
    "midpoints": [[5, 0, 0]],
}

for line in sys.stdin:
    req = json.loads(line)
    rid = req["id"]
    failed = None
    for i, op in enumerate(req["ops"]):
        if op["kind"] == "pushpull" and op["face"] == 666:
            sys.exit(3)
        if op["kind"] == "pushpull" and op["face"] == 999:
            failed = i
            break
    if failed is not None:
        answer({"id": rid, "ok": False, "op": failed, "error": "there is no face 999"})
        continue
    if req["cmd"] == "regen":
        rebuilds += 1
        answer({"id": rid, "ok": True, "mesh": SQUARE, "faces": 1, "new_faces": [0],
                "solids": 0, "volume": rebuilds})
    elif req["cmd"] == "export":
        data = ("FAKE " + req["format"] + " of %d ops\n" % len(req["ops"])).encode()
        with open(req["path"], "wb") as f:
            f.write(data)
        answer({"id": rid, "ok": True, "bytes": len(data)})
