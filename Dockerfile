FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config libopus-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Dependencies (incl. whisper.cpp) are cached in this layer until Cargo.toml/lock change.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS model
ADD https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin /models/ggml-base.en.bin
RUN chmod 644 /models/ggml-base.en.bin

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libopus0 libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home spirit \
    && mkdir -p /data /models && chown spirit /data
COPY --from=model /models /models
COPY --from=builder /app/target/release/spiritbot /usr/local/bin/spiritbot
USER spirit
ENV DATABASE_URL=sqlite:///data/spiritbot.db \
    WHISPER_MODEL=/models/ggml-base.en.bin
VOLUME /data
CMD ["spiritbot"]
