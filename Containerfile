FROM docker.io/library/debian:stable-slim AS certs
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
  rm -rf /var/lib/apt/lists/*

FROM scratch
LABEL maintainer="giggio@giggio.net"
HEALTHCHECK --interval=1m --timeout=3s --start-period=1m --start-interval=5s --retries=3 CMD [ "/cmha", "healthcheck" ]
ENTRYPOINT [ "/cmha" ]
CMD [ "run" ]
ARG target_file=target/output/cmha
COPY --from=certs /etc/ssl/certs /etc/ssl/certs
COPY $target_file /cmha
