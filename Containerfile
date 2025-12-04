FROM scratch
LABEL maintainer="giggio@giggio.net"
ENTRYPOINT [ "/cmha" ]
CMD [ "run" ]
ARG target_file=target/output/cmha
COPY $target_file /cmha
