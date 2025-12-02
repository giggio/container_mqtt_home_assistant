FROM scratch
LABEL maintainer="giggio@giggio.net"
COPY target/output/cmha .
ENTRYPOINT [ "/cmha" ]
