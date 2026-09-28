# Docs

Read, write and find documents written in [Markdown](https://commonmark.org/help/).

## Getting around

The sidebar on the left has three places:

- **📘 Documentation**: how to use each app, including this page. Everyone can read it, and
  nobody can change it from here.
- **🗂 Shared**: everything on the server that you can see.
- **🏠 My documents**: your own folder. Only you can see what is in it. You need to be signed in
  to have one.

Under them is a tree of the place you are in. Press **▸** next to a folder to show what is in
it, and **▾** to hide it again. Click a folder's name to open it, and a document to read it.
The document you are reading is shown in bold, and the tree opens itself to show where you are.

The main page shows the document you picked. For a folder, it shows the folder's `README.md`
if it has one, and the buttons for adding to it. The trail at the top of a folder shows where
you are, and each part of it is a button back to that folder. **← Back** returns to where you
were before.

Docs only shows folders and Markdown (`.md`) documents. Use [Files](file-browser.md) for
everything else.

## Searching

Type in the box at the top of the sidebar and press **🔍**. Docs looks through the names and the text of every
document you can read, and lists each match with the line it was found on.

## Writing

Open a document and press **✏ Edit**. If you see **🔒 Read-only** instead, you can read that
document but not change it.

- Type in the editor. The preview underneath updates as you go; **Hide preview** makes room.
- The buttons above the editor add common Markdown to the end of the document: a heading,
  **bold**, *italic*, a link, lists, a task and a code block.
- **● unsaved** means you have changes that are not saved yet. Press **💾 Save** to keep them,
  then **✔ Done** to go back to reading. Docs will not let you leave with unsaved changes;
  press **🗑 Discard changes** if you do not want them.

To make a new document, open the folder it should go in, type a name next to **Name** and
press **📄 New document**. **📁 New folder** makes a folder the same way. If those buttons are
missing, you cannot add to that folder.

## Markdown you can use

| You type | You get |
|---|---|
| `# Heading`, `## Smaller` | Headings |
| `**bold**`, `*italic*`, `~~struck~~` | **bold**, *italic*, ~~struck~~ |
| `` `code` `` | `code` |
| `- item`, `1. item`, `- [ ] task` | Lists and task lists |
| `> quote` | A quotation |
| `[text](other.md)` | A link to another document, opened in Docs |
| `[text](picture.png)` | A link to any other file, downloaded by your browser |
| `[text](https://example.com)` | A link to a web page, opened in a new tab |
| Three backticks on the lines above and below | A block of code |
| `---` | A dividing line |

Links to other documents can be relative to the current one, such as `../README.md`, or
start from the top with `/`, such as `/Docs/Apps/files.md`.

## Who can see what

Every folder has read and write permissions, and everything inside it gets the same ones
unless a folder further down has its own. Your folder in `/home` is always private to you.
Docs shows you what you are allowed to do, and the server enforces it: Files and every other
app follow the same rules.
