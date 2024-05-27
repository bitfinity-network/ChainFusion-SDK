# Base image
FROM ubuntu:latest

ARG VERSION=0.16.0

# Install necessary packages
RUN apt-get update -y && \
    apt-get install -y curl tar

# Set working directory
WORKDIR /opt

# Download and extract the prebuilt binary for Linux x86_64
RUN curl -L "https://github.com/ordinals/ord/releases/download/0.16.0/ord-0.16.0-x86_64-unknown-linux-gnu.tar.gz" | tar -xz

# Move the ord binary to a directory in the PATH
RUN mv ord-0.16.0/ord /usr/local/bin/ord && \
    rm -rf ord-0.16.0

# Create directory for index data
RUN mkdir -p /index-data

# Expose volume containing all `index-data` data
VOLUME /index-data

# REST interface
EXPOSE 8080

# Set the entrypoint
ENTRYPOINT ["ord"]

CMD ["-r", "--data-dir", "/index-data", "server", "--http-port=8080"]
