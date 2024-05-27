FROM node:18-alpine

WORKDIR /app
RUN apk add curl && npm install -g local-ssl-proxy && curl -JLO "https://dl.filippo.io/mkcert/latest?for=linux/amd64" && chmod +x mkcert-v*-linux-amd64 && cp mkcert-v*-linux-amd64 mkcert
RUN ./mkcert -install && ./mkcert localhost 127.0.0.1 https_proxy