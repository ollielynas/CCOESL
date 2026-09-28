# Modeller

Build 3D models the way you would in SketchUp: draw a shape, then push or pull it into a
solid. The geometry is made on the server by FreeCAD, so the models are exact and can be
opened in other CAD programs.

## Moving around

- **Orbit:** drag with the middle mouse button (or pick **Orbit** and drag with the left one).
- **Pan:** hold **Shift** while you orbit.
- **Zoom:** scroll. The view zooms toward whatever is under the pointer.

The red, green and blue lines are the axes. Everything is measured in millimetres.

## Drawing

- **Rectangle:** click one corner, then the opposite corner. Start on the ground or on any flat
  face of your model.
- **Line:** click each corner of an outline, then click the first corner again to close it.
  The whole outline stays flat on the plane where you started.

While you draw, the pointer snaps to things you can use, and says which:

| You see | It snapped to |
|---|---|
| **Endpoint** (green) | a corner |
| **Midpoint** (cyan) | the middle of an edge |
| **On Red / Green / Blue Axis** | a line from your first corner, straight along that axis |
| **On Face** (blue) | the face under the pointer |

## Push/Pull

Pick **Push/Pull**, then drag a flat face. Pull it out to make a shape taller or add to a
solid; push it in to cut into one. A rectangle drawn on top of a box and pushed in makes a
pocket. The distance shows beside the pointer while you drag, and snaps level with a corner
when you drag over one.

## Exact sizes

After a step, type into **Measurements** and press **Apply**:

- after a push/pull, a distance: `250`, `-30`, `2m`, `40 cm`
- after a rectangle, the width and height: `1200,800` (or `1200;800`)

The last step is remade at that size.

## Undo and redo

**Undo** takes back the last step (or abandons a shape you are halfway through drawing).
**Redo** puts it back. Doing something new after an undo forgets what was undone.

## Saving

Type where to save under **Save as** (your home folder is filled in for you) and press
**STEP** (for other CAD programs), **STL** (for 3D printing) or **FreeCAD** (to carry on in
desktop FreeCAD). When it is saved, **Download** fetches it to this computer.

## If something goes wrong

If FreeCAD cannot do a step, for example a push/pull on a face that no longer exists, the
status line says which step failed. Press **Undo** to go back past it.

**Render** at the top chooses where the view is drawn. **Auto** is right almost always: it
draws in your browser, and hands very large models to the server. Choose **Server** if the
view is slow on this computer.
