FROM mcr.microsoft.com/devcontainers/base:ubuntu

ARG USERNAME=rtm
ARG USER_UID=1000
ARG USER_GID=1000

RUN groupadd --gid "${USER_GID}" "${USERNAME}" \
    && useradd --uid "${USER_UID}" --gid "${USER_GID}" --create-home --shell /bin/bash "${USERNAME}" \
    && mkdir -p /workspace \
    && chown "${USERNAME}:${USERNAME}" /workspace

# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git bash curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
USER ${USERNAME}

CMD ["claude"]
