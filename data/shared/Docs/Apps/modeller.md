# Modeller

Build 3D models the way you would in FreeCAD: sketch a shape, constrain it to exact sizes,
then pad or pocket it into a solid. Or draw a rectangle straight onto a face and push or pull
it, the way SketchUp does. FreeCAD itself does the geometry on the server, so the models are
exact and open in other CAD programs.

## The toolbars

They are laid out like FreeCAD's.

**Top row**, left to right:

- **File:** New, Save (a FreeCAD document), Export as STEP (for other CAD programs), Export
  as STL (for 3D printing). The **File** box below says where they go; your home folder is
  filled in for you. When a file is saved, **Download** fetches it to this computer.
- **Edit:** Undo and Redo. While a sketch is open they undo changes to the sketch.
- **Workbench:** **Part Design** for solids, **Sketcher** for sketches.
- **View:** Fit all, Isometric, and the Front, Top, Right, Rear, Bottom and Left views.
- **Render:** where the view is drawn. **Auto** is right almost always: it draws in your
  browser, and hands very large models to the server. Choose **Server** if the view is slow
  on this computer.

Hover over a button to see what it does.

## Moving around

- **Orbit:** drag with the middle mouse button (or pick **Orbit** and drag with the left one).
- **Pan:** hold **Shift** while you orbit.
- **Zoom:** scroll. The view zooms toward whatever is under the pointer.

The red, green and blue lines are the axes. Everything is measured in millimetres.

## Sketching

Press **Create sketch** (or **Sketcher**), then click the ground or any flat face of the model.
The view turns to face the sketch, and the Sketcher's toolbars appear.

**Geometry:** Point, Line, Polyline, Arc (centre, start, end, counter-clockwise), Circle
(centre, then a point on it) and Rectangle (two corners). While you draw, the pointer snaps to
points and curves. A line drawn nearly level or plumb is made horizontal or vertical, and a
point placed on another point or on a curve is attached there, as in FreeCAD. Right-click or
press Esc to stop part way.

**Editing:** Fillet rounds a corner with the radius in **Value**. Trim cuts away the part of a
curve you click, back to where other curves cross it. Extend lengthens a line or arc to the
next curve. Split cuts a curve in two where you click. External geometry copies an edge of the
model into the sketch, fixed in place. **Construction** switches the selected geometry to
construction lines (blue), which guide the drawing but make no shape. **Offset** copies the
selected geometry the distance in **Value** to one side. The bin deletes what is selected.

**Selecting:** with **Select**, click points, curves and constraint labels to select them,
and click empty space to clear the selection. Drag a point or a curve to move it: the sketch
re-solves as you drag, so everything stays constrained.

**Constraints:** select what to constrain, then press the constraint. The row has FreeCAD's
set, in FreeCAD's order: Coincident, Point on object, Vertical, Horizontal, Parallel,
Perpendicular, Tangent, Equal, Symmetric, Lock, Horizontal distance, Vertical distance,
Distance, Radius, Diameter and Angle. If the selection is not right for it, the status line
says what to select. A dimension starts at the size the geometry already has; to change it,
select its label, type the new value in **Value** (millimetres, or degrees for an angle) and
press **Set**.

The corner of the view says how the sketch stands, as FreeCAD does:

| It says | Meaning |
|---|---|
| **N degrees of freedom** | The geometry can still move in N ways. |
| **Fully constrained** (green) | Nothing can move: every size and position is fixed. |
| **Redundant constraints** (amber) | Some constraints say what others already say. Delete one of those listed. |
| **Conflicting constraints** (red) | Some constraints cannot all hold. Delete one of those listed. |

Press **✔ Close** to finish the sketch, or **✖ Cancel** to leave without keeping changes.
**Edit sketch** opens the most recent sketch again.

## Pad and Pocket

After closing a sketch, type a length in **Measurements** and press **Pad** to pull its closed
shapes out into a solid, or **Pocket** to push them into the solid underneath, cutting it
away. A circle inside a rectangle makes a plate with a hole. With Measurements empty, Pad and
Pocket use 10 mm.

## Quick shapes and Push/Pull

In Part Design, **Rectangle** draws a rectangle with two clicks and **Line** draws any flat
outline (click the first corner again to close it). **Push/Pull** then drags a flat face: pull
it out to make a shape taller or add to a solid; push it in to cut into one. The distance
shows beside the pointer while you drag, and snaps level with a corner when you drag over one.

After a step, type into **Measurements** and press **Apply** to make it exact: a distance
after a push/pull (`250`, `-30`, `2m`, `40 cm`), or width and height after a rectangle
(`1200,800` or `1200;800`).

## If something goes wrong

If FreeCAD cannot do a step, for example a sketch whose constraints conflict, or a push/pull
on a face that no longer exists, the status line says which step failed. Press **Undo** to go
back past it.

The first time anyone uses the Modeller, the server may need to install FreeCAD. The status
line shows its progress; it takes a minute or two, and only happens once.
