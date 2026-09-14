#!/usr/bin/env bash
# Runs a command against disposable OpenSSH/PostgreSQL containers; always cleans up.
set -euo pipefail
cd "$(dirname "$0")"
if (($# == 0)); then
  echo 'Usage: scripts/ssh-fixture/run.sh COMMAND [ARG...]' >&2
  exit 2
fi
command -v docker >/dev/null
for prerequisite in ssh-keygen ssh-agent ssh-add openssl python3; do
  command -v "$prerequisite" >/dev/null
done
docker info >/dev/null
fixture=$(mktemp -d)
name="queri-ssh-$(id -u)-$$"
cleanup() {
  [[ -z ${agent_pid:-} ]] || kill "$agent_pid" 2>/dev/null || true
  docker rm -fv "$name-ssh" "$name-pg" >/dev/null 2>&1 || true
  docker network rm "$name" >/dev/null 2>&1 || true
  rm -rf "$fixture"
}
trap cleanup EXIT
ssh-keygen -q -t ed25519 -N '' -f "$fixture/host_key"
ssh-keygen -q -t ed25519 -N '' -f "$fixture/plain_key"
ssh-keygen -q -t ed25519 -N 'fixture-passphrase' -f "$fixture/encrypted_key"
ssh-keygen -q -t rsa -b 2048 -m PEM -N '' -f "$fixture/rsa_pem_key"
ssh-keygen -q -t ecdsa -b 256 -N '' -f "$fixture/ecdsa_key"
openssl pkcs8 -topk8 -in "$fixture/rsa_pem_key" -out "$fixture/rsa_pkcs8_key" \
  -v2 aes256 -passout pass:fixture-passphrase
cat "$fixture/plain_key.pub" "$fixture/encrypted_key.pub" "$fixture/rsa_pem_key.pub" "$fixture/ecdsa_key.pub" > "$fixture/authorized_keys"
# Only ephemeral fixture material is mounted; never use developer SSH keys.
chmod 755 "$fixture"
chmod 644 "$fixture/authorized_keys"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=Queri-Fixture-CA \
  -keyout "$fixture/ca.key" -out "$fixture/ca.crt" >/dev/null 2>&1
openssl req -new -newkey rsa:2048 -nodes -subj /CN=database \
  -keyout "$fixture/server.key" -out "$fixture/server.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:database\nextendedKeyUsage=serverAuth\n' > "$fixture/server.ext"
openssl x509 -req -in "$fixture/server.csr" -CA "$fixture/ca.crt" -CAkey "$fixture/ca.key" \
  -CAcreateserial -days 1 -extfile "$fixture/server.ext" -out "$fixture/server.crt" >/dev/null 2>&1
export SSL_CERT_FILE="$fixture/ca.crt"
export SSH_AUTH_SOCK="$fixture/agent.sock"
ssh-agent -a "$SSH_AUTH_SOCK" -D >"$fixture/agent.log" 2>&1 &
agent_pid=$!
for ((attempt=0; attempt<50; attempt++)); do
  [[ ! -S $SSH_AUTH_SOCK ]] || break
  sleep .1
done
ssh-add "$fixture/plain_key"
docker build -t queri-ssh-fixture .
docker network create "$name" >/dev/null
docker run -d --name "$name-pg" --network "$name" --network-alias database --network-alias wrong-database \
  -e POSTGRES_PASSWORD=fixture-password -e POSTGRES_DB=fixture -v "$fixture:/fixture:ro" postgres:18-alpine \
  sh -c 'cp /fixture/server.key /tmp/server.key; cp /fixture/server.crt /tmp/server.crt; chown postgres /tmp/server.key; chmod 600 /tmp/server.key; exec docker-entrypoint.sh postgres -c ssl=on -c ssl_key_file=/tmp/server.key -c ssl_cert_file=/tmp/server.crt' >/dev/null
ssh_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
docker run -d --name "$name-ssh" --network "$name" \
  -p "127.0.0.1:$ssh_port:22" -v "$fixture:/fixture:ro" queri-ssh-fixture >/dev/null
ready=false
for ((attempt=0; attempt<60; attempt++)); do
  if docker exec "$name-pg" pg_isready -U postgres -d fixture >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ $ready != true ]]; then
  docker logs "$name-pg" >&2
  exit 1
fi
export QUERI_SSH_HOST=127.0.0.1
QUERI_SSH_PORT=$(docker port "$name-ssh" 22/tcp | sed 's/.*://')
export QUERI_SSH_PORT
export QUERI_SSH_USER=tunnel
export QUERI_SSH_KEY="$fixture/plain_key"
export QUERI_SSH_ENCRYPTED_KEY="$fixture/encrypted_key"
export QUERI_SSH_RSA_KEY="$fixture/rsa_pem_key"
export QUERI_SSH_PKCS8_KEY="$fixture/rsa_pkcs8_key"
export QUERI_SSH_ECDSA_KEY="$fixture/ecdsa_key"
export QUERI_SSH_PASSPHRASE=fixture-passphrase
export QUERI_SSH_HOST_KEY="$fixture/host_key"
export QUERI_SSH_HOST_PUBLIC_KEY="$fixture/host_key.pub"
export QUERI_SSH_DATABASE_HOST=database
export QUERI_SSH_DATABASE_PORT=5432
export QUERI_SSH_DATABASE_PASSWORD=fixture-password
export QUERI_SSH_CONTAINER="$name-ssh"
export QUERI_SSH_DATABASE_CONTAINER="$name-pg"
# Database has no published port. Its DNS alias exists only on this network.
"$@"
