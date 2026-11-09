#!/bin/sh
# In-container forwarder: the CLIs speak plain HTTP to 127.0.0.1:8080, which is the only
# way out: a Unix socket the host bind-mounts at /run/kwnt/broker.sock.
socat TCP-LISTEN:8080,bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:/run/kwnt/broker.sock &
exec sleep infinity
