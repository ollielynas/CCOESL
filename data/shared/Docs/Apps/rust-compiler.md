# Compiler

Build a Rust, C or C++ project on the server, and download the program it makes.

## Choosing a project

There are two ways to pick what to build:

- **Upload one from this computer.** Press **⬆ Upload project** and choose the project's folder.
  Anything its `.gitignore` leaves out, such as `target/`, is not uploaded. The upload is only
  kept for building: the server deletes it after an hour unused.
- **Use one already on the server.** Click folders to open them, and **⬆ Up** to go back.

## What can be built

When you are in a folder that can be built, the app says what it found there:

| It finds | It runs |
|---|---|
| **Rust project**: a `Cargo.toml` | `cargo build --release` |
| **C/C++ project**: a `Makefile` (or `makefile`) | `make` |
| **C/C++ sources**: `.c`, `.cpp`, `.cc` or `.cxx` files, and no build file | each `.c` file through `gcc` and each C++ file through `g++`, then links them into one program |

If a folder has more than one of these, the first in the table wins: a folder with a `Makefile`
is built the way its `Makefile` says.

For plain **C/C++ sources**, only the files directly in the folder are compiled, not ones in
folders inside it. Headers (`.h`) can be anywhere `#include` finds them. The program is named
after the folder and goes in its `target/` folder, along with the compiled pieces. The maths
library is linked in, so `#include <math.h>` works.

For a **Makefile** project, the programs listed when it finishes are the ones `make` made or
changed.

Programs are built for the server, not for the computer you're using. They run on computers
like the server (the same operating system and processor).

## Building

Press **🔨 Build**. A build can take a few minutes. The progress bar and the step it is on
update while it runs, and you can look around other folders in the meantime without stopping
it.

When it finishes, the app says whether it worked and shows the compiler's output, including
every error. For a build that worked, each program it made is listed with its size and a
**Download** button.

Building runs whatever the project says to run, including everything in a `Makefile`. Only
build projects you trust.

You can only build projects you are allowed to read. Read about
[who can see what](docs.md) in the Docs page.
