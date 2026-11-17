# kwant on Render (render.yaml). Render mounts its persistent disk on /data, and a fresh disk
# may belong to root, while kwnt runs as uid 10001. So this starts as root only long enough to
# hand /data to kwnt, then runs kwnt as uid 10001. Nothing is compiled: it is the released image.
FROM ghcr.io/alexsergivan/kwnt:0.9.17@sha256:8ce673fe74fe309d408f0a93b3b55659d263a0aa5abbe17a202ffa07a7f62ae3
USER root
COPY render-entrypoint.sh /usr/local/bin/kwnt-render-entrypoint
RUN chmod 755 /usr/local/bin/kwnt-render-entrypoint
ENTRYPOINT ["/usr/local/bin/kwnt-render-entrypoint"]
CMD ["kwnt", "serve"]
