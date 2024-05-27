#!/usr/bin/env bash

local-ssl-proxy --source 8001 --target 8000 --key localhost+3-key.pem --cert localhost+3.pem &
./ord -r --bitcoin-rpc-url=http://bitcoind:18443 server --http-port=8000
