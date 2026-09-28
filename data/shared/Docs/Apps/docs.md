# Docs

Read, write and find documents written in [Markdown](https://commonmark.org/help/).

## Getting around

The sidebar on the left has three places. The one you are in is highlighted.

- **:book_open: Documentation**: how to use each app, including this page. Everyone can read
  it, and nobody can change it from here.
- **:users: Shared**: everything on the server that you can see.
- **:house: My documents**: your own folder. Only you can see what is in it. You need to be
  signed in to have one.

Under them is a tree of the place you are in. Click a folder to open it: the page shows it, and
the tree lists what is inside, with the folder's icon open (:folder_open:). Click the folder
you are in again to close it (:folder:). Click a document (:file_text:) to read it. The row for
what you are looking at is highlighted, and the tree opens itself to show where you are.

The sidebar and the page scroll separately, so a long document never moves the tree.

When you pick a folder, the page shows its `README.md` if it has one, and the buttons for
adding to it. The trail at the top shows where you are, starting from the place, and each part
of it is a button back there. **:arrow_left: Back** returns to where you were before.

Docs only shows folders and Markdown (`.md`) documents. Use [Files](file-browser.md) for
everything else.

## Searching

Type in the box at the top of the sidebar and press **:magnifying_glass:**. Docs looks through
the names and the text of every document you can read, and lists each match with the line it
was found on.

## Writing

Open a document and press **:pencil_simple: Edit**. If you see **:lock_simple: Read-only**
instead, you can read that document but not change it.

- Type in the editor. The preview underneath updates as you go; **:eye_slash: Hide preview**
  makes room.
- The buttons above the editor add common Markdown to the end of the document: a heading
  (:text_h:), **bold** (:text_b:), *italic* (:text_italic:), a link (:link:), lists
  (:list_bullets: :list_numbers:), a task (:check_square:) and a code block (:code_block:).
- **• Unsaved changes** means there are changes that are not saved yet. Press
  **:floppy_disk: Save** to keep them, then **:check: Done** to go back to reading. Docs will
  not let you leave with unsaved changes; press **:trash: Discard changes** if you do not want
  them.

To make a new document, open the folder it should go in, type a name next to **Name** and
press **:file_plus: New document**. **:folder_plus: New folder** makes a folder the same way.
If those buttons are missing, you cannot add to that folder.

## Markdown you can use

| You type | You get |
|---|---|
| `# Heading`, `## Smaller` | Headings |
| `**bold**`, `*italic*`, `~~struck~~` | **bold**, *italic*, ~~struck~~ |
| `` `code` `` | `code` |
| `- item`, `1. item`, `- [ ] task` | Lists and task lists |
| `> quote` | A quotation |
| `:house:`, `:folder-open:` | An icon: :house: :folder_open: |
| `[text](other.md)` | A link to another document, opened in Docs |
| `[text](picture.png)` | A link to any other file, downloaded by your browser |
| `[text](https://example.com)` | A link to a web page, opened in a new tab |
| Three backticks on the lines above and below | A block of code |
| `---` | A dividing line |

Links to other documents can be relative to the current one, such as `../README.md`, or
start from the top with `/`, such as `/Docs/Apps/files.md`.

## Icons

Put the name of any [Phosphor icon](https://phosphoricons.com) between colons, the way Discord
does emoji: `:rocket:` gives :rocket:, and `:warning_circle:` gives :warning_circle:. Use the
name shown on the Phosphor site, with `-` or `_` between words (`:folder-open:` and
`:folder_open:` both work). A name that is not an icon stays as you typed it, and nothing
inside `` `code` `` is changed.

## Who can see what

Every folder has read and write permissions, and everything inside it gets the same ones
unless a folder further down has its own. Your folder in `/home` is always private to you.
Docs shows you what you are allowed to do, and the server enforces it: Files and every other
app follow the same rules.
