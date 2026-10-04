# syntax=docker/dockerfile:1
#
# CCOSEL in one container: the server, the web shell and apps, and the Keycloak that signs
# people in. No config files: state lives in the /data volume, and the server sets Keycloak up
# on first start. See "Running it in Docker" in README.md.
#
#   docker build -t ccosel .
#   docker run -d --name ccosel -p 8777:8777 -v ccosel-data:/data ccosel

# ---- build: the server binary and web/dist ---------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src
# node runs the pinned wasm-opt, if `cargo xtask build-web` uses it.
RUN apt-get update && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*
COPY . .
# The toolchain comes from rust-toolchain.toml; wasm-bindgen-cli must match the crate's pin.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    rustup show active-toolchain \
    && version="$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' Cargo.toml)" \
    && test -n "$version" \
    && cargo install wasm-bindgen-cli --version "$version" --locked
# The build caches below are shared by every checkout built on this machine (all at /src), and
# cargo trusts a cached artifact that is newer than its source. So another checkout's build of
# an older version of one of our crates, made after this checkout's source was last edited,
# would be reused here, silently: the image would hold that checkout's code, not this one's
# (#92). And `COPY . .` keeps each file's mtime from whichever build first copied that content.
# Touching the sources makes this build's the newest, so every workspace crate is compiled
# from them; crates.io dependencies are untouched and stay cached. `sharing=locked` stops a
# concurrent build writing an older artifact between the touch and the compile.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,sharing=locked \
    --mount=type=cache,target=/src/apps/target,sharing=locked \
    find . -path ./target -prune -o -path ./apps/target -prune -o -type f -exec touch {} + \
    && cargo xtask build-web \
    && cargo build --release -p ccosel-server \
    && cp target/release/ccosel-server /usr/local/bin/ccosel-server

# ---- run ---------------------------------------------------------------------------------------
FROM eclipse-temurin:21-jre-noble
# Keycloak, pinned. It runs as a child of the server (CCOSEL_KEYCLOAK_HOME), not in Docker.
COPY --from=quay.io/keycloak/keycloak:26.7.4 /opt/keycloak /opt/keycloak

# The Compiler app runs `cargo build` on the server. Without a toolchain that app can't build
# anything, but everything else works: `--build-arg WITH_RUST=0` makes the image ~1 GB smaller.
ARG WITH_RUST=1
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
# GNU Go is the Go app's opponent.
RUN apt-get update \
    && apt-get install -y --no-install-recommends gnugo \
    && if [ "$WITH_RUST" = 1 ]; then \
         apt-get install -y --no-install-recommends ca-certificates curl gcc libc6-dev \
         && curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path \
              --target wasm32-unknown-unknown \
         && chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"; \
       fi \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local/bin/ccosel-server /usr/local/bin/ccosel-server
COPY --from=build /src/web /srv/ccosel/web
# The Docs app's pages. A new named volume starts with these; an existing one keeps its own.
COPY --from=build /src/data/shared/Docs /data/files/Docs

# Everything that must survive the container: files people keep, Keycloak's accounts, and the
# Keycloak admin password (under $XDG_DATA_HOME/ccosel).
RUN useradd --create-home --uid 10001 ccosel \
    && mkdir -p /data/files /data/keycloak \
    && rm -rf /opt/keycloak/data && ln -s /data/keycloak /opt/keycloak/data \
    && chown -R ccosel:ccosel /data /opt/keycloak
USER ccosel
VOLUME /data
ENV XDG_DATA_HOME=/data \
    CCOSEL_KEYCLOAK_HOME=/opt/keycloak \
    CCOSEL_KEYCLOAK_LISTEN=0.0.0.0
EXPOSE 8777
# Keycloak's admin console, for adding accounts. Publish it to this machine only:
# `-p 127.0.0.1:8080:8080`. Sign-in pages don't need it; they go through 8777 under /idp.
EXPOSE 8080
CMD ["ccosel-server", "--root", "/data/files", "--web", "/srv/ccosel/web", "--port", "8777"]
