# kwant on Render (render.yaml). Render mounts its persistent disk on /data, and a fresh disk
# may belong to root, while kwnt runs as uid 10001. So this starts as root only long enough to
# hand /data to kwnt, then runs kwnt as uid 10001. Nothing is compiled: it is the released image.
FROM ghcr.io/alexsergivan/kwnt:0.9.56@sha256:174ec968b26c15311e02707a4b825701ccac1d0d3e274591d1b6f432d83c5deb
USER root
COPY render-entrypoint.sh /usr/local/bin/kwnt-render-entrypoint
RUN chmod 755 /usr/local/bin/kwnt-render-entrypoint
ENTRYPOINT ["/usr/local/bin/kwnt-render-entrypoint"]
CMD ["kwnt", "serve"]
