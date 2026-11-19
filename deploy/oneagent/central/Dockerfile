# OneHub Central: build multi-estágio reproduzível (Rust no próprio contexto do repositório, Cargo.lock fixado).
# Contexto de build = raiz do repositório. Imagem final mínima, usuário sem privilégios, sem shell de root.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p one-central

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home oneagent
COPY --from=build /src/target/release/one-central /usr/local/bin/one-central
COPY crates/one-central/migrations /opt/oneagent/migrations
ENV ONEAGENT_CENTRAL_MIGRATIONS=/opt/oneagent/migrations
USER oneagent
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/one-central"]
