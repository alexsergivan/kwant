#!/bin/sh
# The container's only way out: the model API and kwnt's own tools through kwnt's broker, a
# Unix socket kwnt bind-mounts read-only at /run/kwnt. The CLI speaks plain HTTP to 127.0.0.1:8080.
socat TCP-LISTEN:8080,bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:/run/kwnt/broker.sock &
exec sleep infinity
