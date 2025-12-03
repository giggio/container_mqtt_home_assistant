FROM scratch
LABEL maintainer="giggio@giggio.net"
ARG target_file=target/output/cmha
COPY $target_file /cmha
ENTRYPOINT [ "/cmha" ]
