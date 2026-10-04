# Files

Browse the folders on the server, and move files between it and your computer.

## Shared and My files

The two tabs at the top are the two places your files can be:

- **Shared** is what is shared on the server: folders other people can see too, as far as
  each folder's permissions allow.
- **My files** is your private folder. Only you can see what is in it, and nobody else can
  change it. You need to be signed in to have one; until you are, the tab says so instead.

Switching tabs takes you to the top of that place. **⬆ Up** and the trail at the top never go
above it.

## Getting around

- Click a folder to open it. **⬆ Up** goes to the folder above.
- The trail at the top shows where you are. Click any part of it to jump back there.
- **⟳ Refresh** lists the folder again, for when someone else has changed it.
- Type in the **🔍 Filter** box to show only the names that contain what you typed.

Clicking a file selects it, and its name is shown at the bottom.

## Opening, sharing and downloading a file

Right-click a file's name for what you can do with it:

- **Open with Viewer** shows it in the [Viewer](viewer.md): pictures, music, videos, PDFs and
  text files.
- **Share with Viewer** copies a link that opens the file in the Viewer. Whoever follows it signs
  in first, and sees the file only if they are allowed to read it.
- **Download** saves it to your computer.

## Uploading

- **⬆ Upload files** asks your browser to pick one or more files on your computer, and uploads
  them into the folder you are looking at.
- **⬆ Upload folder** picks a whole folder instead, and uploads all of it, folders inside it
  included.
- You can also drag files or folders from your computer onto the window.

The list refreshes when the upload finishes.

## Compressing and extracting

Right-click a file or folder for these, in a folder you can change:

- **Compress (.tar.gz)** bundles a file or a whole folder into one `.tar.gz` file next to it,
  such as `photos.tar.gz` for the folder `photos`. It's smaller to download and keeps folders
  together.
- **Gzip (.gz)** compresses a single file on its own, such as `notes.txt.gz`.
- **Extract here** appears on `.tar`, `.tar.gz`, `.tgz` and `.gz` files. A `.tar.gz` unpacks
  into a new folder named after it (`photos.tar.gz` makes `photos`), and a `.gz` turns back into
  the file it holds.

The work happens on the server, so a big folder doesn't have to come down to your computer and
back. A line above the list says how far it has got, and what it made once it's done. You can
keep browsing meanwhile. Only one runs at a time.

Nothing is ever overwritten. If the name is taken, the new one gets a number: `photos (2)`.

Permissions files are never put into an archive, and an archive holding one is refused.
Extracting is refused, and leaves nothing behind, if the archive:

- tries to put files outside the new folder;
- holds links that lead outside it;
- would unpack to more than 4 GB, or more than 100,000 files.

`.zip` and other formats aren't supported yet.

## Deleting

Right-click a file or folder you can change and choose **Delete**. Files asks first, because
a deletion can't be undone: answer **Delete it** to go ahead, or **Cancel**. Deleting a folder
deletes everything in it.

A folder can only be deleted if you can change everything inside it, so a folder holding one
you may only read stays put, along with what is in it. Read-only files and folders have no
**Delete** at all.

## Permissions

Under the trail, a line says what you can do in the folder you are looking at:

- **Can change**: you can add, change and delete files here, and the upload buttons are shown.
- **Read-only**: you can open and download files here, but not change them. There are no
  upload buttons.

The **Permissions** column says the same for each file and folder in the list.

## What you can see

You only ever see files and folders you are allowed to read. Anything else is left out of the
list completely, including a shortcut (link) that leads somewhere you can't read. If you try
to open something you can't read, Files says it isn't there, just as if it didn't exist, so
nobody can find out what is in another person's folder, or even what it is called.

Read about [who can see what](docs.md#who-can-see-what) in the Docs page.
