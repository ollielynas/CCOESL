# Your files as a drive (WebDAV)

You can open the server's files straight from your own computer, as a network drive in Finder,
Windows Explorer or your Linux file manager, without the desktop in the browser. You see the
same folders you see in **Files**, and the same rules apply: what you can read or change here,
you can read or change there, and nothing more.

## 1. Make an app password

A drive cannot sign in through the sign-in page, so it uses an **app password** instead: a
password for one device, separate from your real one.

1. Open **Account**.
2. Under **App passwords**, type a name for the device, such as `Laptop`, and press **Create**.
3. Copy the password it shows. **It is shown only once.** If you lose it, revoke it and make a
   new one.

Make one per device. If a device is lost or you stop using it, press **Revoke** next to its
name and it stops working straight away. Your real password is never involved, so it does not
need changing.

If the server has login turned off, skip this step: there is nothing to sign in to, and
everyone gets the folders that are open to everyone.

## 2. Connect

**Account** shows the address under **App passwords**, next to **Connect to**, with a
**Copy address** button. It is the server's address with `/dav/` on the end, for example
`https://ccosel.example.com/dav/`. Sign in with your usual user name and the app password.

**Use an `https://` address.** Over plain `http://`, the password is sent across the network
where anyone on it could read it, and Windows refuses to send it at all. If your server is
only reachable over `http://`, ask whoever runs it to put it behind HTTPS.

### macOS (Finder)

1. In Finder, choose **Go → Connect to Server…** (⌘K).
2. Enter the address, for example `https://ccosel.example.com/dav/`, and press **Connect**.
3. Choose **Registered User**, enter your user name and the app password, and press
   **Connect**.

### Windows (File Explorer)

1. Right-click **This PC** and choose **Map network drive…**.
2. Pick a drive letter and, under **Folder**, enter the address, for example
   `https://ccosel.example.com/dav/`.
3. Tick **Connect using different credentials**, press **Finish**, then enter your user name
   and the app password.

### Linux

- **GNOME Files (Nautilus):** choose **Other Locations**, and under **Connect to Server**
  enter the address with `davs://` in place of `https://`, for example
  `davs://ccosel.example.com/dav/`.
- **KDE Dolphin:** enter `webdavs://ccosel.example.com/dav/` in the location bar.
- **davfs2**, to mount it as a folder:

  ```sh
  sudo mount -t davfs https://ccosel.example.com/dav/ /mnt/ccosel
  ```

### rclone

```sh
rclone config create ccosel webdav url=https://ccosel.example.com/dav/ vendor=other \
  user=your-name pass="$(rclone obscure 'the app password')"
rclone ls ccosel:
```

## What you can do

| You can… | when… |
|---|---|
| See a folder or file | you can read it. Folders you cannot read are not listed at all. |
| Open or copy a file | you can read it. |
| Save, add or make a folder | you can change the folder it goes in. |
| Delete something | you can change it, the folder it is in, and **everything inside it**. A folder holding anything you cannot change, or cannot see, cannot be deleted. |
| Move something | you could delete it, and you can change where it is going. |
| Copy a folder | you can read **everything inside it**. If any of it is hidden from you, the copy is refused rather than made without it. |
| Replace something already there | you could delete what is there. |

Something you move or copy takes on the rules of the folder it lands in.

Your own folder is at `home/your-name`. You can do anything inside it, but not delete or move
the folder itself.

Each file can be at most 64 MB, the same limit as uploading in **Files**.

## If it will not connect

- **It keeps asking for the password.** Check you are using an app password, not your real
  one, that it has not been revoked, and that your user name is spelled as when you sign in.
- **"Too many requests" or it refuses for a few minutes.** After ten wrong passwords in a row,
  the server stops checking for five minutes, for that user name and for that address. Wait,
  then try again with the right one.
- **Windows says the folder is not valid.** Make sure the address starts with `https://`.
