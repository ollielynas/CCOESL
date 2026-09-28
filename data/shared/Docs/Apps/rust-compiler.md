# Compiler

Build a Rust project on the server, and download the program it makes.

## Choosing a project

There are two ways to pick what to build:

- **Upload one from this computer.** Press **⬆ Upload project** and choose the project's folder.
  Anything its `.gitignore` leaves out, such as `target/`, is not uploaded. The upload is only
  kept for building: the server deletes it after an hour unused.
- **Use one already on the server.** Click folders to open them, and **⬆ Up** to go back.

A folder can be built when it has a `Cargo.toml` in it. The app says so when you are in one.

## Building

Press **🔨 Build**. A build can take a few minutes. The progress bar and the step it is on
update while it runs, and you can look around other folders in the meantime without stopping
it.

When it finishes, the app says whether it worked and shows the compiler's output. For a build
that worked, each program it made is listed with its size and a **Download** button.

You can only build projects you are allowed to read. Read about
[who can see what](docs.md) in the Docs page.
