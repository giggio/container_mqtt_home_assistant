FROM scratch
LABEL maintainer="giggio@giggio.net"
# todo: change to released binary
COPY target/debug/cmha .
ENTRYPOINT [ "/cmha" ]
