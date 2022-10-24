FROM rust as builder
WORKDIR /usr/src/kruidvat-notification
COPY . .
RUN cargo install --path .

CMD ["kruidvat-notification"]
