# Opening one app on its own

Every app has its own address that opens just that app in a browser tab, filling the page with no
desktop, dock or windows around it. It is handy for a bookmark, for keeping one app on a second
screen, or for sending someone straight to the app they need.

| App | Its own page |
|---|---|
| Files | [/app/file-browser](/app/file-browser) |
| Clock | [/app/clock](/app/clock) |
| Server | [/app/server-dashboard](/app/server-dashboard) |
| Compiler | [/app/rust-compiler](/app/rust-compiler) |
| Account | [/app/account](/app/account) |
| Docs | [/app/docs](/app/docs) |
| Viewer | [/app/viewer](/app/viewer) |

Each link opens in a new tab. To share one, put the server's address in front, for example
`http://192.168.1.20:8777/app/clock`.

- The app works the same as it does in a window, and it signs in the same way: if the server
  asks you to sign in, it does so before the app opens.
- Files you drop anywhere on the page go to the app, the same as dropping them on its window.
- The Viewer's page can open a file straight away: **Share** in the Viewer, or **Share with
  Viewer** in Files, copies a link like `/app/viewer?open=%2FPhotos%2Fbeach.jpg`.
- To get back to the desktop, go to the server's address with nothing after it.
