FROM rust:1.42.0-slim-buster

RUN apt-get update \
  && apt-get install -y --no-install-recommends clang=1:7.* cmake=3.* \
     libsnappy-dev=1.* \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/*

RUN adduser --disabled-login --system --shell /bin/false --uid 1000 user

USER user
WORKDIR /home/user
COPY ./ /home/user

RUN cargo build --release
RUN cargo install --path .

# Electrum RPC
EXPOSE 50001

# Prometheus monitoring
EXPOSE 4224

STOPSIGNAL SIGINT
