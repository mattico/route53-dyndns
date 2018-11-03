FROM rust

# Create blank project with Cargo.{toml,lock} so we can cache dependencies
WORKDIR /usr/src
RUN USER=root cargo new route53-dyndns
WORKDIR /usr/src/route53-dyndns
COPY Cargo.toml Cargo.lock /usr/src/route53-dyndns/
RUN cargo build --release && cargo clean --release -p route53-dyndns

# Build the source
COPY src /usr/src/route53-dyndns/src
RUN cargo install --path .

ENV RUST_BACKTRACE=1
CMD ["/usr/local/cargo/bin/route53-dyndns"]
