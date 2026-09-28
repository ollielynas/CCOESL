"""The CCOSEL CAD worker: FreeCAD, driven over stdin/stdout.

The server starts one of these with `freecadcmd` and keeps it running, so FreeCAD loads
once. Each request is one JSON line on stdin; each answer is one line on stdout starting
with `@@CCOSEL `, so anything FreeCAD itself prints is ignored.

A model is a list of operations. Rebuilding replays them, but every prefix already built is
cached, so adding an operation to a long model costs one operation, and undoing costs
nothing.

Requests:
  {"id": 1, "cmd": "regen", "ops": [...]}
      -> {"id": 1, "ok": true, "mesh": {...}, "faces": n, "solids": n, "volume": v}
      -> {"id": 1, "ok": false, "op": i, "error": "..."}
  {"id": 2, "cmd": "export", "ops": [...], "format": "step"|"stl"|"fcstd", "path": "/abs"}
      -> {"id": 2, "ok": true, "bytes": n}

Operations:
  {"kind": "polygon", "points": [[x, y, z], ...], "normal": [x, y, z]}
  {"kind": "pushpull", "after": k, "face": f, "distance": d}
"""

import json
import os
import sys
import traceback
from collections import OrderedDict

import FreeCAD
import Part

TOL = 1e-6
CACHE_LIMIT = 256


def vec(p):
    return FreeCAD.Vector(float(p[0]), float(p[1]), float(p[2]))


def lst(v):
    return [float(v.x), float(v.y), float(v.z)]


class Model:
    """The shapes a model is made of: solids, and loose faces not yet pushed or pulled."""

    def __init__(self, shapes=()):
        self.shapes = list(shapes)

    def faces(self):
        """Every face, numbered in this order. The mesh uses the same numbering."""
        out = []
        for s in self.shapes:
            out.extend(s.Faces)
        return out


def is_loose(shape):
    return shape.ShapeType == "Face"


def flat_normal(face):
    """The outward normal of a flat face, or None for a curved one."""
    if not isinstance(face.Surface, Part.Plane):
        return None
    u0, u1, v0, v1 = face.ParameterRange
    n = face.normalAt((u0 + u1) / 2, (v0 + v1) / 2)
    return n.normalize()


def on_boundary(solid, face):
    """Whether `face` lies on the surface of `solid`: what makes a push/pull cut into it or
    build out from it, rather than start a new solid."""
    if solid.distToShape(face)[0] > 1e-5:
        return False
    c = face.CenterOfMass
    return solid.isInside(c, 1e-5, True) and not solid.isInside(c, 1e-5, False)


def polygon(model, op):
    pts = [vec(p) for p in op["points"]]
    clean = []
    for p in pts:
        if not clean or (p - clean[-1]).Length > TOL:
            clean.append(p)
    if len(clean) > 1 and (clean[0] - clean[-1]).Length <= TOL:
        clean.pop()
    if len(clean) < 3:
        raise ValueError("a face needs at least three distinct corners")
    face = Part.Face(Part.makePolygon(clean + [clean[0]]))
    if not face.isValid() or face.Area <= TOL:
        raise ValueError("the outline crosses itself or has no area")
    n = flat_normal(face)
    if n is not None and n.dot(vec(op["normal"])) < 0:
        face.reverse()
    return Model(model.shapes + [face])


def push_pull(model, op, history):
    after = int(op["after"])
    if after < 0 or after >= len(history):
        raise ValueError("push/pull refers to a model that does not exist yet")
    faces = history[after].faces()
    index = int(op["face"])
    if index < 0 or index >= len(faces):
        raise ValueError("there is no face %d" % index)
    face = faces[index]
    n = flat_normal(face)
    if n is None:
        raise ValueError("only flat faces can be pushed or pulled")
    d = float(op["distance"])
    if abs(d) <= TOL:
        return Model(model.shapes)
    prism = face.extrude(n * d)

    shapes = [s for s in model.shapes if not (is_loose(s) and s.isSame(face))]
    for i, s in enumerate(shapes):
        if is_loose(s) or not s.Solids or not on_boundary(s, face):
            continue
        result = s.fuse(prism) if d > 0 else s.cut(prism)
        result = result.removeSplitter()
        if result.Solids:
            shapes[i] = result
        else:
            del shapes[i]
        return Model(shapes)
    shapes.append(prism)
    return Model(shapes)


def op_key(ops, n):
    return json.dumps(ops[:n], sort_keys=True)


class Builder:
    def __init__(self):
        self.cache = OrderedDict()

    def remember(self, key, model):
        self.cache[key] = model
        self.cache.move_to_end(key)
        while len(self.cache) > CACHE_LIMIT:
            self.cache.popitem(last=False)

    def build(self, ops):
        """The model after every operation, as a list: history[k] is after k operations."""
        history = [Model()]
        start = 0
        # The longest prefix already built.
        for n in range(len(ops), 0, -1):
            key = op_key(ops, n)
            if key in self.cache:
                # Rebuild the history list up to n from cached prefixes.
                prefix = []
                for k in range(1, n + 1):
                    prefix.append(self.cache.get(op_key(ops, k)))
                if all(m is not None for m in prefix):
                    history = [Model()] + prefix
                    start = n
                    for k in range(1, n + 1):
                        self.cache.move_to_end(op_key(ops, k))
                break
        for i in range(start, len(ops)):
            op = ops[i]
            try:
                kind = op.get("kind")
                if kind == "polygon":
                    model = polygon(history[-1], op)
                elif kind == "pushpull":
                    model = push_pull(history[-1], op, history)
                else:
                    raise ValueError("unknown operation %r" % kind)
            except Exception as e:  # noqa: BLE001 - every failure is reported, not fatal
                raise OpError(i, str(e) or type(e).__name__)
            history.append(model)
            self.remember(op_key(ops, i + 1), model)
        return history


class OpError(Exception):
    def __init__(self, op, message):
        super().__init__(message)
        self.op = op
        self.message = message


def tessellate(model):
    shapes = model.shapes
    bound = None
    for s in shapes:
        bound = s.BoundBox if bound is None else bound.united(s.BoundBox)
    tol = max(0.01, (bound.DiagonalLength if bound else 1.0) * 0.001)

    positions, triangles, tri_face, normals, loose = [], [], [], [], []
    edges, vertices, midpoints = [], [], []
    face_id = 0
    for s in shapes:
        for f in s.Faces:
            n = flat_normal(f)
            pts, tris = f.tessellate(tol)
            base = len(positions)
            positions.extend(lst(p) for p in pts)
            for t in tris:
                a, b, c = t
                if n is not None:
                    tn = (pts[b] - pts[a]).cross(pts[c] - pts[a])
                    if tn.dot(n) < 0:
                        b, c = c, b
                triangles.append([base + a, base + b, base + c])
                tri_face.append(face_id)
            normals.append(lst(n) if n is not None else [0.0, 0.0, 0.0])
            loose.append(is_loose(s))
            face_id += 1
        for e in s.Edges:
            pts = e.discretize(Deflection=tol)
            for a, b in zip(pts, pts[1:]):
                edges.append([lst(a), lst(b)])
            mid = (e.FirstParameter + e.LastParameter) / 2
            midpoints.append(lst(e.valueAt(mid)))
        vertices.extend(lst(v.Point) for v in s.Vertexes)

    solids = sum(len(s.Solids) for s in shapes)
    volume = sum(sol.Volume for s in shapes for sol in s.Solids)
    return {
        "mesh": {
            "positions": positions,
            "triangles": triangles,
            "tri_face": tri_face,
            "face_normals": normals,
            "loose": loose,
            "edges": edges,
            "vertices": vertices,
            "midpoints": midpoints,
        },
        "faces": face_id,
        "solids": solids,
        "volume": volume,
    }


def export(model, fmt, path):
    shapes = model.shapes
    if not shapes:
        raise ValueError("the model is empty")
    compound = Part.makeCompound(shapes)
    if fmt == "step":
        compound.exportStep(path)
    elif fmt == "stl":
        compound.exportStl(path)
    elif fmt == "fcstd":
        doc = FreeCAD.newDocument("ccosel_export")
        try:
            for i, s in enumerate(shapes):
                obj = doc.addObject("Part::Feature", "Shape%d" % i)
                obj.Shape = s
            doc.recompute()
            doc.saveAs(path)
        finally:
            FreeCAD.closeDocument(doc.Name)
    else:
        raise ValueError("unknown format %r" % fmt)
    return os.path.getsize(path)


def answer(msg):
    sys.stdout.write("@@CCOSEL " + json.dumps(msg) + "\n")
    sys.stdout.flush()


def handle(builder, req):
    rid = req.get("id")
    ops = req.get("ops", [])
    try:
        history = builder.build(ops)
    except OpError as e:
        return {"id": rid, "ok": False, "op": e.op, "error": e.message}
    model = history[-1]
    cmd = req.get("cmd")
    if cmd == "regen":
        out = tessellate(model)
        out.update({"id": rid, "ok": True})
        return out
    if cmd == "export":
        size = export(model, req.get("format"), req.get("path"))
        return {"id": rid, "ok": True, "bytes": size}
    return {"id": rid, "ok": False, "op": -1, "error": "unknown command %r" % cmd}


def main():
    builder = Builder()
    answer({"ready": True, "version": ".".join(FreeCAD.Version()[:3])})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except ValueError:
            answer({"id": None, "ok": False, "op": -1, "error": "malformed request"})
            continue
        try:
            answer(handle(builder, req))
        except Exception as e:  # noqa: BLE001 - the worker must survive any one request
            traceback.print_exc(file=sys.stderr)
            answer({"id": req.get("id"), "ok": False, "op": -1, "error": str(e)})


main()
