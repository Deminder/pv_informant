FROM clux/muslrust as builder
WORKDIR /usr/src/pv_informant
COPY Cargo.toml .
COPY src src
RUN cargo install --path .

FROM debian:buster-slim
RUN apt-get update && apt-get install -y iproute2 inetutils-ping && rm -rf /var/lib/apt/lists/*
COPY --from=builder /root/.cargo/bin/pv_informant /usr/local/bin/pv_informant
ENV INFLUXDB_CLIENT=user:password@http://127.0.0.1:8086:dbname
CMD ["pv_informant"]
