FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=${USER_UID}

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    clojure \
    cmake \
    curl \
    fonts-liberation \
    git \
    jq \
    libasound2t64 \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libcups2 \
    libdrm2 \
    libgbm1 \
    libgtk-3-0 \
    libnss3 \
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    libx11-xcb1 \
    libxcomposite1 \
    libxdamage1 \
    libxfixes3 \
    libxkbcommon0 \
    libxrandr2 \
    maven \
    openjdk-21-jdk-headless \
    pkg-config \
    python3 \
    sudo \
    unzip \
    zip \
  && rm -rf /var/lib/apt/lists/* \
  && groupadd --force --gid "${USER_GID}" "${USERNAME}" \
  && if ! id -u "${USERNAME}" >/dev/null 2>&1; then \
    useradd --uid "${USER_UID}" --non-unique --gid "${USER_GID}" -m -s /bin/bash "${USERNAME}"; \
  fi \
  && echo "${USERNAME} ALL=(root) NOPASSWD:ALL" > "/etc/sudoers.d/${USERNAME}" \
  && chmod 0440 "/etc/sudoers.d/${USERNAME}"

USER ${USERNAME}
WORKDIR /workspaces/hara
