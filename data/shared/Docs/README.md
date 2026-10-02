# CCOSEL documentation

Welcome. CCOSEL is a desktop that runs in your browser. Apps open in windows, and your files
live on the server rather than on the computer you are using, so they are there from any
machine on the network.

## Apps

- [Files](Apps/file-browser.md): browse the server's folders, and upload and download files
- [Docs](Apps/docs.md): read, write and search documents, including these ones
- [Clock](Apps/clock.md): a clock and stopwatch
- [Server](Apps/server-dashboard.md): live graphs of how the server is doing
- [Compiler](Apps/rust-compiler.md): build a Rust project on the server and download the result
- [Account](Apps/account.md): see who you are signed in as, sign out, and make app passwords
- [Viewer](Apps/viewer.md): look at pictures, music, videos, PDFs and text files, and share them

You can also [open your files as a drive](WebDAV.md) on your own computer, with WebDAV.

Any app can also be opened on its own, in a tab with no desktop around it. The
[links to each app's own page](app-links.md) are in one place.

## Where your files go

| Folder | Who can see it |
|---|---|
| `/home/your-name` | Only you. Everyone who signs in gets one. Open it with **🏠 My documents** in Docs. |
| `/Docs` | Everyone, read-only. The developers keep it up to date. |
| Everything else | Depends on the folder. Open it in **Docs** to see whether you can add to it. |

A folder's permissions apply to everything inside it, so a new document gets the permissions
of the folder you create it in.
