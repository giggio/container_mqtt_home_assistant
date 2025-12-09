FROM scratch
LABEL maintainer="giggio@giggio.net"
HEALTHCHECK --interval=1m --timeout=3s --start-period=1m --start-interval=5s --retries=3 CMD [ "/cmha", "healthcheck" ]
ENTRYPOINT [ "/cmha" ]
CMD [ "run" ]
ARG target_file=target/output/cmha
COPY $target_file /cmha
