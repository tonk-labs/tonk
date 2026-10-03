ARG BASE_IMAGE
FROM ${BASE_IMAGE}
RUN if command -v apk >/dev/null; then \
      apk add --no-cache binutils; \
    else \
      apt-get update && apt-get install -y --no-install-recommends binutils \
      && rm -rf /var/lib/apt/lists/*; \
    fi
