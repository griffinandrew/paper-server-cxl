FROM rust:1.88.0 AS builder

# the working directory inside the container
WORKDIR /usr/src/paper

COPY Cargo.toml Cargo.lock default.pconf log4rs.yaml ./
COPY ./src ./src

# create a release build
RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /usr/src/paper

COPY --from=builder /usr/src/paper/target/release/paper-server ./

# run the server
ENTRYPOINT ["/usr/src/paper/paper-server"]
