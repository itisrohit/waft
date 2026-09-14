FROM rust:1.94-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --features internet --bin waft-signaling

FROM debian:bookworm-slim
COPY --from=build /src/target/release/waft-signaling /usr/local/bin/waft-signaling
EXPOSE 8787
ENV WAFT_SIGNALING_BIND=0.0.0.0:8787
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/waft-signaling"]
