# syntax=docker/dockerfile:1
FROM rust:1.96-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets
COPY templates ./templates
ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=owlytics-target-${TARGETARCH},target=/build/target,sharing=locked \
    cargo build --release --locked && cp target/release/owlytics /build/owlytics

FROM debian:bookworm-slim
RUN groupadd --gid 10001 owlytics \
    && useradd --uid 10001 --gid owlytics --no-create-home --shell /usr/sbin/nologin owlytics \
    && mkdir /data \
    && chown owlytics:owlytics /data
COPY --from=build /build/owlytics /usr/local/bin/owlytics
COPY LICENSE /usr/share/doc/owlytics/LICENSE
COPY docs/licenses/ /usr/share/doc/owlytics/licenses/
USER 10001:10001
WORKDIR /data
ENV OWLYTICS_BIND=0.0.0.0:3000 \
    OWLYTICS_DATABASE=/data/owlytics.sqlite \
    SQLITE_TMPDIR=/data
EXPOSE 3000
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/owlytics"]
