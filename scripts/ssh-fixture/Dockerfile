FROM alpine:3.23
RUN apk add --no-cache openssh-server postgresql-client && \
    adduser -D tunnel && passwd -u tunnel
COPY sshd_config /etc/ssh/sshd_config
CMD ["/usr/sbin/sshd", "-D", "-e"]
