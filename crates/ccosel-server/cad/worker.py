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
  {"kind": "sketch", "sketch": {...}}   a ccosel_proto::sketch::Sketch, as serde writes it

A sketch becomes a real Sketcher::SketchObject, solved by FreeCAD's own solver; its closed
profiles become loose faces, ready to push or pull like a drawn rectangle.
"""

import json
import os
import sys
import traceback
from collections import OrderedDict

import FreeCAD
import Part
import Sketcher

TOL = 1e-6
CACHE_LIMIT = 256


def vec(p):
    return FreeCAD.Vector(float(p[0]), float(p[1]), float(p[2]))


def lst(v):
    return [float(v.x), float(v.y), float(v.z)]


class Model:
    """The shapes a model is made of: solids, and loose faces not yet pushed or pulled.

    `added` is how many shapes at the end the last operation made, whose faces are the new
    faces Pad and Pocket act on; `sketches` are the sketches so far, for a FreeCAD export.
    """

    def __init__(self, shapes=(), added=0, sketches=()):
        self.shapes = list(shapes)
        self.added = added
        self.sketches = list(sketches)

    def new_faces(self):
        before = sum(len(s.Faces) for s in self.shapes[: len(self.shapes) - self.added])
        return list(range(before, sum(len(s.Faces) for s in self.shapes)))

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
    return Model(model.shapes + [face], 1, model.sketches)


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
        return Model(model.shapes, 0, model.sketches)
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
        return Model(shapes, 0, model.sketches)
    shapes.append(prism)
    return Model(shapes, 0, model.sketches)


POS = {"Edge": 0, "Start": 1, "End": 2, "Mid": 3}

_work_doc = None


def work_doc():
    """A document to build sketches in. Sketcher objects need one."""
    global _work_doc
    if _work_doc is None:
        _work_doc = FreeCAD.newDocument("ccosel_work")
    return _work_doc


def plane_placement(plane):
    x = vec(plane["x_dir"]).normalize()
    n = vec(plane["normal"]).normalize()
    y = n.cross(x)
    m = FreeCAD.Matrix(x.x, y.x, n.x, 0, x.y, y.y, n.y, 0, x.z, y.z, n.z, 0, 0, 0, 0, 1)
    return FreeCAD.Placement(vec(plane["origin"]), FreeCAD.Rotation(m))


def v2(p):
    return FreeCAD.Vector(float(p[0]), float(p[1]), 0)


def geometry(curve):
    (kind, val), = curve.items()
    if kind == "Point":
        return Part.Point(v2(val))
    if kind == "Line":
        return Part.LineSegment(v2(val[0]), v2(val[1]))
    circle = Part.Circle(v2(val["center"]), FreeCAD.Vector(0, 0, 1), float(val["radius"]))
    if kind == "Circle":
        return circle
    return Part.ArcOfCircle(circle, float(val["start"]), float(val["end"]))


def pt(r):
    return [int(r["geo"]), POS[r["pos"]]]


def constraint(c):
    """A ccosel constraint as FreeCAD's Sketcher.Constraint arguments."""
    (kind, v), = c.items()
    if kind == "Coincident":
        return ["Coincident"] + pt(v[0]) + pt(v[1])
    if kind == "PointOnObject":
        return ["PointOnObject"] + pt(v[0]) + [int(v[1])]
    if kind in ("Horizontal", "Vertical"):
        return [kind, int(v)]
    if kind in ("HorizontalPoints", "VerticalPoints"):
        return [kind.replace("Points", "")] + pt(v[0]) + pt(v[1])
    if kind in ("Parallel", "Perpendicular", "Tangent", "Equal"):
        return [kind, int(v[0]), int(v[1])]
    if kind == "Symmetric":
        return ["Symmetric"] + pt(v[0]) + pt(v[1]) + [int(v[2])]
    if kind == "SymmetricPoint":
        return ["Symmetric"] + pt(v[0]) + pt(v[1]) + pt(v[2])
    if kind == "Distance":
        return ["Distance", int(v[0]), float(v[1])]
    if kind == "DistancePoints":
        return ["Distance"] + pt(v[0]) + pt(v[1]) + [float(v[2])]
    if kind == "DistancePointLine":
        return ["Distance"] + pt(v[0]) + [int(v[1]), float(v[2])]
    if kind in ("DistanceX", "DistanceY"):
        return [kind] + pt(v[0]) + pt(v[1]) + [float(v[2])]
    if kind in ("Radius", "Diameter"):
        return [kind, int(v[0]), float(v[1])]
    if kind == "Angle":
        return ["Angle", int(v[0]), float(v[1])]
    if kind == "AngleBetween":
        return ["Angle", int(v[0]), int(v[1]), float(v[2])]
    raise ValueError("unknown constraint %r" % kind)


def sketch_object(doc, sk, name="Sketch"):
    """`sk` as a Sketcher::SketchObject in `doc`, solved by FreeCAD."""
    obj = doc.addObject("Sketcher::SketchObject", name)
    obj.Placement = plane_placement(sk["plane"])
    for g in sk["geos"]:
        obj.addGeometry(geometry(g["curve"]), bool(g["construction"]))
    cons = [Sketcher.Constraint(*constraint(c)) for c in sk["constraints"]]
    cons += [Sketcher.Constraint("Block", i) for i, g in enumerate(sk["geos"]) if g["fixed"]]
    if cons:
        obj.addConstraint(cons)
    rc = obj.solve()
    if rc == -2:
        raise ValueError("the sketch has redundant constraints")
    if rc == -3:
        raise ValueError("the sketch has conflicting constraints")
    if rc != 0:
        raise ValueError(
            "FreeCAD could not solve the sketch: its constraints conflict or cannot all hold"
        )
    doc.recompute()
    return obj


def sketch_op(model, op):
    sk = op["sketch"]
    doc = work_doc()
    obj = sketch_object(doc, sk)
    try:
        shape = obj.Shape.copy()
    finally:
        doc.removeObject(obj.Name)
    closed = [w for w in shape.Wires if w.isClosed()]
    faces = []
    if closed:
        made = Part.makeFace(closed, "Part::FaceMakerBullseye")
        n = vec(sk["plane"]["normal"])
        for f in made.Faces:
            fn = flat_normal(f)
            if fn is not None and fn.dot(n) < 0:
                f.reverse()
            faces.append(f)
    return Model(model.shapes + faces, len(faces), model.sketches + [sk])


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
                elif kind == "sketch":
                    model = sketch_op(history[-1], op)
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
        "new_faces": model.new_faces(),
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
    if not shapes and not (fmt == "fcstd" and model.sketches):
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
            # The sketches too, editable in FreeCAD's Sketcher.
            for i, sk in enumerate(model.sketches):
                sketch_object(doc, sk, "Sketch%d" % i)
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
