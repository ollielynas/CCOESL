//! The constraint solver.
//!
//! Every geometry is a handful of numbers (a line is two points, an arc a centre, radius and
//! two angles) and every constraint is one or two equations in them that are zero when it
//! holds. Solving is Gauss–Newton with the **least-change** step: of all the moves that would
//! satisfy the equations to first order, take the smallest. So an under-constrained sketch
//! moves as little as it can, which is what makes dragging feel right, and what FreeCAD's own
//! solver does.
//!
//! The Jacobian is by central differences: sketches have tens of unknowns, and hand-written
//! derivatives for twenty kinds of constraint would be twenty places to get a sign wrong.
//!
//! Degrees of freedom are the unknowns minus the rank of the Jacobian at the solution. A
//! constraint whose equations add nothing to the rank of the ones before it is **redundant**
//! when the sketch still solves, and **conflicting** when it does not — FreeCAD's two warnings.

use std::f64::consts::PI;

use ccosel_proto::sketch::{Constraint, Curve, H_AXIS, PointRef, Pos, Sketch, V_AXIS};

use crate::geom::{P2, cross, dist, dot, len, normalize, polar, sub};

/// Equations closer to zero than this (mm, or radians) hold.
const TOL: f64 = 1e-9;
const MAX_ITER: usize = 200;

/// How a solve went.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Every constraint holds.
    pub converged: bool,
    /// Ways the geometry can still move without breaking a constraint.
    pub dof: usize,
    /// Constraints already implied by the others (indices into `constraints`).
    pub redundant: Vec<usize>,
    /// Constraints that cannot hold together with the others.
    pub conflicting: Vec<usize>,
    /// Constraints that name geometry of the wrong kind, and are ignored.
    pub malformed: Vec<usize>,
}

impl Report {
    /// FreeCAD's green: nothing left to move and nothing wrong.
    pub fn fully_constrained(&self) -> bool {
        self.converged && self.dof == 0 && self.conflicting.is_empty()
    }
}

fn nparams(c: &Curve) -> usize {
    match c {
        Curve::Point(_) => 2,
        Curve::Line(..) => 4,
        Curve::Circle { .. } => 3,
        Curve::Arc { .. } => 5,
    }
}

fn pack(c: &Curve, out: &mut Vec<f64>) {
    match *c {
        Curve::Point(p) => out.extend(p),
        Curve::Line(a, b) => out.extend([a[0], a[1], b[0], b[1]]),
        Curve::Circle { center, radius } => out.extend([center[0], center[1], radius]),
        Curve::Arc {
            center,
            radius,
            start,
            end,
        } => out.extend([center[0], center[1], radius, start, end]),
    }
}

fn unpack(c: &Curve, x: &[f64]) -> Curve {
    match c {
        Curve::Point(_) => Curve::Point([x[0], x[1]]),
        Curve::Line(..) => Curve::Line([x[0], x[1]], [x[2], x[3]]),
        Curve::Circle { .. } => Curve::Circle {
            center: [x[0], x[1]],
            radius: x[2],
        },
        Curve::Arc { .. } => Curve::Arc {
            center: [x[0], x[1]],
            radius: x[2],
            start: x[3],
            end: x[4],
        },
    }
}

/// The unknowns of a sketch: every geometry's numbers end to end.
struct System {
    off: Vec<usize>,
    x: Vec<f64>,
    /// Which entries of `x` the solver may change.
    free: Vec<bool>,
    /// For each tangency between two circles, whether it is internal. Decided once from the
    /// sketch as it was, so a solve cannot flip from one to the other half way.
    internal: Vec<bool>,
}

impl System {
    fn new(sk: &Sketch) -> Self {
        let mut off = Vec::with_capacity(sk.geos.len());
        let mut x = Vec::new();
        let mut free = Vec::new();
        for g in &sk.geos {
            off.push(x.len());
            pack(&g.curve, &mut x);
            free.resize(x.len(), !g.fixed);
        }
        let mut sys = Self {
            off,
            x,
            free,
            internal: Vec::new(),
        };
        sys.internal = sk
            .constraints
            .iter()
            .map(|c| match *c {
                Constraint::Tangent(a, b) => {
                    let e = Eval {
                        sk,
                        sys: &sys,
                        x: &sys.x,
                    };
                    match (e.circle(a), e.circle(b)) {
                        (Some((c1, r1)), Some((c2, r2))) => dist(c1, c2) < r1.max(r2),
                        _ => false,
                    }
                }
                _ => false,
            })
            .collect();
        sys
    }

    fn write_back(&self, sk: &mut Sketch) {
        for (i, g) in sk.geos.iter_mut().enumerate() {
            let o = self.off[i];
            g.curve = normalize(unpack(&g.curve, &self.x[o..o + nparams(&g.curve)]));
        }
    }

    fn vars(&self) -> Vec<usize> {
        (0..self.x.len()).filter(|&i| self.free[i]).collect()
    }
}

/// A sketch's geometry read through a trial vector of unknowns.
struct Eval<'a> {
    sk: &'a Sketch,
    sys: &'a System,
    x: &'a [f64],
}

impl Eval<'_> {
    fn params(&self, g: i32) -> Option<(&Curve, &[f64])> {
        let i = usize::try_from(g).ok()?;
        let c = &self.sk.geos.get(i)?.curve;
        let o = self.sys.off[i];
        Some((c, &self.x[o..o + nparams(c)]))
    }

    fn point(&self, r: PointRef) -> Option<P2> {
        if r.geo == H_AXIS || r.geo == V_AXIS {
            return (r.pos == Pos::Start).then_some([0.0, 0.0]);
        }
        let (c, p) = self.params(r.geo)?;
        match (c, r.pos) {
            (Curve::Point(_), Pos::Start | Pos::Mid) => Some([p[0], p[1]]),
            (Curve::Line(..), Pos::Start) => Some([p[0], p[1]]),
            (Curve::Line(..), Pos::End) => Some([p[2], p[3]]),
            (Curve::Circle { .. } | Curve::Arc { .. }, Pos::Mid) => Some([p[0], p[1]]),
            (Curve::Arc { .. }, Pos::Start) => Some(polar([p[0], p[1]], p[2], p[3])),
            (Curve::Arc { .. }, Pos::End) => Some(polar([p[0], p[1]], p[2], p[4])),
            _ => None,
        }
    }

    /// A line as a point and a direction (the axes included).
    fn line(&self, g: i32) -> Option<(P2, P2)> {
        match g {
            H_AXIS => Some(([0.0, 0.0], [1.0, 0.0])),
            V_AXIS => Some(([0.0, 0.0], [0.0, 1.0])),
            _ => match self.params(g)? {
                (Curve::Line(..), p) => Some(([p[0], p[1]], [p[2] - p[0], p[3] - p[1]])),
                _ => None,
            },
        }
    }

    fn circle(&self, g: i32) -> Option<(P2, f64)> {
        match self.params(g)? {
            (Curve::Circle { .. } | Curve::Arc { .. }, p) => Some(([p[0], p[1]], p[2])),
            _ => None,
        }
    }
}

/// `a` wrapped into `(-π, π]`.
fn wrap_pi(a: f64) -> f64 {
    let w = (a + PI).rem_euclid(2.0 * PI) - PI;
    if w <= -PI { w + 2.0 * PI } else { w }
}

/// Signed distance from `p` to the line through `o` along `d`.
fn line_distance(o: P2, d: P2, p: P2) -> f64 {
    cross(d, sub(p, o)) / len(d).max(1e-300)
}

/// Append constraint `ci`'s equations to `out`; `false` if it names the wrong kind of geometry.
fn equations(e: &Eval, ci: usize, c: &Constraint, out: &mut Vec<f64>) -> bool {
    let start = out.len();
    let ok = (|| -> Option<()> {
        match *c {
            Constraint::Coincident(a, b) => {
                let (p, q) = (e.point(a)?, e.point(b)?);
                out.extend([p[0] - q[0], p[1] - q[1]]);
            }
            Constraint::PointOnObject(a, g) => {
                let p = e.point(a)?;
                if let Some((o, d)) = e.line(g) {
                    out.push(line_distance(o, d, p));
                } else {
                    let (cc, r) = e.circle(g)?;
                    out.push(dist(p, cc) - r);
                }
            }
            Constraint::Horizontal(g) => out.push(e.line(g).filter(|_| g >= 0)?.1[1]),
            Constraint::Vertical(g) => out.push(e.line(g).filter(|_| g >= 0)?.1[0]),
            Constraint::HorizontalPoints(a, b) => out.push(e.point(a)?[1] - e.point(b)?[1]),
            Constraint::VerticalPoints(a, b) => out.push(e.point(a)?[0] - e.point(b)?[0]),
            Constraint::Parallel(a, b) => {
                let (d1, d2) = (e.line(a)?.1, e.line(b)?.1);
                out.push(cross(d1, d2) / (len(d1) * len(d2)).max(1e-300));
            }
            Constraint::Perpendicular(a, b) => {
                let (d1, d2) = (e.line(a)?.1, e.line(b)?.1);
                out.push(dot(d1, d2) / (len(d1) * len(d2)).max(1e-300));
            }
            Constraint::Tangent(a, b) => match (e.line(a), e.line(b)) {
                // Two lines are tangent when they are one line.
                (Some((o1, d1)), Some((o2, d2))) => {
                    out.push(cross(d1, d2) / (len(d1) * len(d2)).max(1e-300));
                    out.push(line_distance(o1, d1, o2));
                }
                (Some((o, d)), None) | (None, Some((o, d))) => {
                    let (cc, r) = e.circle(if e.line(a).is_some() { b } else { a })?;
                    out.push(line_distance(o, d, cc).abs() - r);
                }
                (None, None) => {
                    let ((c1, r1), (c2, r2)) = (e.circle(a)?, e.circle(b)?);
                    let reach = if e.sys.internal[ci] {
                        (r1 - r2).abs()
                    } else {
                        r1 + r2
                    };
                    out.push(dist(c1, c2) - reach);
                }
            },
            Constraint::Equal(a, b) => match (e.line(a), e.line(b)) {
                (Some((_, d1)), Some((_, d2))) => out.push(len(d1) - len(d2)),
                _ => out.push(e.circle(a)?.1 - e.circle(b)?.1),
            },
            Constraint::Symmetric(a, b, g) => {
                let (p, q) = (e.point(a)?, e.point(b)?);
                let (o, d) = e.line(g)?;
                let m = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
                out.push(line_distance(o, d, m));
                out.push(dot(sub(q, p), d) / len(d).max(1e-300));
            }
            Constraint::SymmetricPoint(a, b, m) => {
                let (p, q, c) = (e.point(a)?, e.point(b)?, e.point(m)?);
                out.extend([(p[0] + q[0]) / 2.0 - c[0], (p[1] + q[1]) / 2.0 - c[1]]);
            }
            Constraint::Distance(g, v) => out.push(len(e.line(g).filter(|_| g >= 0)?.1) - v),
            Constraint::DistancePoints(a, b, v) => out.push(dist(e.point(a)?, e.point(b)?) - v),
            Constraint::DistancePointLine(a, g, v) => {
                let (o, d) = e.line(g)?;
                out.push(line_distance(o, d, e.point(a)?).abs() - v);
            }
            Constraint::DistanceX(a, b, v) => out.push(e.point(b)?[0] - e.point(a)?[0] - v),
            Constraint::DistanceY(a, b, v) => out.push(e.point(b)?[1] - e.point(a)?[1] - v),
            Constraint::Radius(g, v) => out.push(e.circle(g)?.1 - v),
            Constraint::Diameter(g, v) => out.push(2.0 * e.circle(g)?.1 - v),
            Constraint::Angle(g, v) => {
                let d = e.line(g).filter(|_| g >= 0)?.1;
                out.push(wrap_pi(d[1].atan2(d[0]) - v));
            }
            Constraint::AngleBetween(a, b, v) => {
                let (d1, d2) = (e.line(a)?.1, e.line(b)?.1);
                out.push(wrap_pi(cross(d1, d2).atan2(dot(d1, d2)) - v));
            }
        }
        Some(())
    })()
    .is_some();
    if !ok {
        out.truncate(start);
    }
    ok
}

/// All equations, and which constraint each row belongs to.
fn residuals(sk: &Sketch, sys: &System, x: &[f64]) -> (Vec<f64>, Vec<usize>) {
    let e = Eval { sk, sys, x };
    let mut r = Vec::new();
    let mut owner = Vec::new();
    for (i, c) in sk.constraints.iter().enumerate() {
        let before = r.len();
        equations(&e, i, c, &mut r);
        owner.resize(owner.len() + (r.len() - before), i);
    }
    (r, owner)
}

/// ∂r/∂x over the free unknowns, by central differences. Row-major, `m × vars.len()`.
fn jacobian(sk: &Sketch, sys: &System, x: &mut [f64], vars: &[usize], m: usize) -> Vec<f64> {
    let n = vars.len();
    let mut j = vec![0.0; m * n];
    for (col, &v) in vars.iter().enumerate() {
        let x0 = x[v];
        let h = 1e-7 * x0.abs().max(1.0);
        x[v] = x0 + h;
        let (rp, _) = residuals(sk, sys, x);
        x[v] = x0 - h;
        let (rm, _) = residuals(sk, sys, x);
        x[v] = x0;
        for row in 0..m.min(rp.len()).min(rm.len()) {
            j[row * n + col] = (rp[row] - rm[row]) / (2.0 * h);
        }
    }
    j
}

/// Solve `a y = b` for a square `a` (row-major), by elimination with partial pivoting.
fn solve_linear(mut a: Vec<f64>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let m = b.len();
    for col in 0..m {
        let piv =
            (col..m).max_by(|&i, &k| a[i * m + col].abs().total_cmp(&a[k * m + col].abs()))?;
        if a[piv * m + col].abs() < 1e-300 {
            return None;
        }
        if piv != col {
            for k in 0..m {
                a.swap(col * m + k, piv * m + k);
            }
            b.swap(col, piv);
        }
        for row in col + 1..m {
            let f = a[row * m + col] / a[col * m + col];
            if f != 0.0 {
                for k in col..m {
                    a[row * m + k] -= f * a[col * m + k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    let mut y = vec![0.0; m];
    for row in (0..m).rev() {
        let mut s = b[row];
        for k in row + 1..m {
            s -= a[row * m + k] * y[k];
        }
        y[row] = s / a[row * m + row];
    }
    Some(y)
}

fn sq(r: &[f64]) -> f64 {
    r.iter().map(|v| v * v).sum()
}

/// Levenberg–Marquardt in its least-change form, from `sys.x`. `true` when every equation
/// holds.
///
/// Each step solves `(J Jᵀ + μI) y = r` and moves by `-Jᵀ y`. With a small `μ` that is the
/// Gauss–Newton step that changes the geometry least; when a step fails to bring the equations
/// closer to holding, `μ` grows and the step turns towards plain descent, which gets out of the
/// places (a point crossing a line it must keep a distance from, say) where Newton stalls.
fn newton(sk: &Sketch, sys: &mut System) -> bool {
    let vars = sys.vars();
    let mut lambda = 1e-6;
    for _ in 0..MAX_ITER {
        let (r, _) = residuals(sk, sys, &sys.x);
        if r.iter().all(|v| v.abs() < TOL) {
            return true;
        }
        if vars.is_empty() {
            return false;
        }
        let m = r.len();
        let n = vars.len();
        let mut x = sys.x.clone();
        let mut j = jacobian(sk, sys, &mut x, &vars, m);
        // Each equation divided by the size of its gradient, so a dimensionless one (an angle,
        // a parallel) counts as much as a length. The weights are fixed for this step, and
        // the step is judged by the same weighted error it descends.
        let w: Vec<f64> = (0..m)
            .map(|p| {
                let g = (0..n)
                    .map(|k| j[p * n + k] * j[p * n + k])
                    .sum::<f64>()
                    .sqrt();
                1.0 / g.max(1e-9)
            })
            .collect();
        for p in 0..m {
            for k in 0..n {
                j[p * n + k] *= w[p];
            }
        }
        let rn: Vec<f64> = r.iter().zip(&w).map(|(v, w)| v * w).collect();
        let mut jjt = vec![0.0; m * m];
        for p in 0..m {
            for q in p..m {
                let s: f64 = (0..n).map(|k| j[p * n + k] * j[q * n + k]).sum();
                jjt[p * m + q] = s;
                jjt[q * m + p] = s;
            }
        }
        let before = sq(&rn);
        let mut improved = false;
        while lambda < 1e10 {
            let mut a = jjt.clone();
            for p in 0..m {
                a[p * m + p] += lambda;
            }
            if let Some(y) = solve_linear(a, rn.clone()) {
                let mut trial = sys.x.clone();
                for (k, &v) in vars.iter().enumerate() {
                    trial[v] -= (0..m).map(|p| j[p * n + k] * y[p]).sum::<f64>();
                }
                let (rt, _) = residuals(sk, sys, &trial);
                let after = rt
                    .iter()
                    .zip(&w)
                    .map(|(v, w)| (v * w) * (v * w))
                    .sum::<f64>();
                if rt.len() == m && trial.iter().all(|v| v.is_finite()) && after < before {
                    sys.x = trial;
                    lambda = (lambda / 10.0).max(1e-12);
                    improved = true;
                    break;
                }
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    let (r, _) = residuals(sk, sys, &sys.x);
    r.iter().all(|v| v.abs() < TOL * 1e3)
}

/// Degrees of freedom, and which constraints add nothing to the ones before them.
fn analyse(sk: &Sketch, sys: &System, converged: bool) -> Report {
    let vars = sys.vars();
    let (r, owner) = residuals(sk, sys, &sys.x);
    let m = r.len();
    let n = vars.len();
    let mut x = sys.x.clone();
    let j = jacobian(sk, sys, &mut x, &vars, m);

    let mut basis: Vec<Vec<f64>> = Vec::new();
    let mut dependent = Vec::new();
    for row in 0..m {
        let mut v: Vec<f64> = j[row * n..(row + 1) * n].to_vec();
        let norm0 = v.iter().map(|a| a * a).sum::<f64>().sqrt();
        if norm0 < 1e-12 {
            if !dependent.contains(&owner[row]) {
                dependent.push(owner[row]);
            }
            continue;
        }
        for a in v.iter_mut() {
            *a /= norm0;
        }
        for b in &basis {
            let d: f64 = v.iter().zip(b).map(|(p, q)| p * q).sum();
            for (p, q) in v.iter_mut().zip(b) {
                *p -= d * q;
            }
        }
        let norm = v.iter().map(|a| a * a).sum::<f64>().sqrt();
        if norm > 1e-6 {
            for a in v.iter_mut() {
                *a /= norm;
            }
            basis.push(v);
        } else if !dependent.contains(&owner[row]) {
            dependent.push(owner[row]);
        }
    }
    let malformed = (0..sk.constraints.len())
        .filter(|i| !owner.contains(i))
        .collect();
    let (redundant, conflicting) = if converged {
        (dependent, Vec::new())
    } else {
        (Vec::new(), dependent)
    };
    Report {
        converged,
        dof: n - basis.len(),
        redundant,
        conflicting,
        malformed,
    }
}

/// Move the geometry to satisfy every constraint, as little as possible. A sketch that cannot
/// be solved is left as it was.
pub fn solve(sk: &mut Sketch) -> Report {
    let mut sys = System::new(sk);
    let converged = newton(sk, &mut sys);
    if converged {
        sys.write_back(sk);
        let sys = System::new(sk);
        analyse(sk, &sys, true)
    } else {
        let sys = System::new(sk);
        analyse(sk, &sys, false)
    }
}

/// What the pointer is dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    Point(PointRef),
    /// A curve by its edge: a line moves bodily, a circle or arc changes radius.
    Edge(i32),
}

/// Drag `grab` from `from` to `to` (sketch coordinates) and re-solve. What was grabbed follows
/// the pointer exactly if the constraints allow; otherwise it goes as near as they let it.
pub fn drag(sk: &mut Sketch, grab: Grab, from: P2, to: P2) -> Report {
    let geo = match grab {
        Grab::Point(r) => r.geo,
        Grab::Edge(g) => g,
    };
    let Some(g) = usize::try_from(geo).ok().filter(|&g| g < sk.geos.len()) else {
        return solve(sk);
    };
    if sk.geos[g].fixed {
        return solve(sk);
    }
    let mut sys = System::new(sk);
    let o = sys.off[g];
    let pinned: Vec<usize> = match (sk.geos[g].curve, grab) {
        (Curve::Point(_), _) => {
            sys.x[o] = to[0];
            sys.x[o + 1] = to[1];
            vec![o, o + 1]
        }
        (Curve::Line(..), Grab::Point(r)) if r.pos == Pos::Start || r.pos == Pos::End => {
            let k = if r.pos == Pos::Start { o } else { o + 2 };
            sys.x[k] = to[0];
            sys.x[k + 1] = to[1];
            vec![k, k + 1]
        }
        (Curve::Line(..), _) => {
            let d = sub(to, from);
            for k in [o, o + 2] {
                sys.x[k] += d[0];
                sys.x[k + 1] += d[1];
            }
            vec![o, o + 1, o + 2, o + 3]
        }
        (Curve::Circle { .. } | Curve::Arc { .. }, Grab::Point(r)) if r.pos == Pos::Mid => {
            sys.x[o] = to[0];
            sys.x[o + 1] = to[1];
            vec![o, o + 1]
        }
        (Curve::Arc { .. }, Grab::Point(r)) => {
            let k = if r.pos == Pos::Start { o + 3 } else { o + 4 };
            let angle = (to[1] - sys.x[o + 1]).atan2(to[0] - sys.x[o]);
            // Keep the angle continuous, so the arc does not jump a whole turn.
            sys.x[k] += wrap_pi(angle - sys.x[k]);
            vec![k]
        }
        (Curve::Circle { .. } | Curve::Arc { .. }, Grab::Edge(_)) => {
            sys.x[o + 2] = dist(to, [sys.x[o], sys.x[o + 1]]);
            vec![o + 2]
        }
        _ => Vec::new(),
    };
    let moved = sys.x.clone();
    for &k in &pinned {
        sys.free[k] = false;
    }
    if newton(sk, &mut sys) {
        sys.write_back(sk);
        return analyse(sk, &System::new(sk), true);
    }
    // Pinned it cannot be: let it go, and settle as close to the pointer as the rest allows.
    let mut sys = System::new(sk);
    sys.x = moved;
    if newton(sk, &mut sys) {
        sys.write_back(sk);
        return analyse(sk, &System::new(sk), true);
    }
    analyse(sk, &System::new(sk), false)
}

/// The value a dimensional constraint would hold for the geometry as it is: what a new
/// dimension is created with.
pub fn measure(sk: &Sketch, c: &Constraint) -> Option<f64> {
    c.value()?;
    let sys = System::new(sk);
    let e = Eval {
        sk,
        sys: &sys,
        x: &sys.x,
    };
    let zero = c.with_value(0.0);
    let mut out = Vec::new();
    if !equations(&e, 0, &zero, &mut out) {
        return None;
    }
    // With the value zero, a dimension's equation is exactly what it measures.
    out.first().copied()
}

#[cfg(test)]
mod tests;
