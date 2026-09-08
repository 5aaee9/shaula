# Supply a reviewed official Runner image by its immutable digest.
ARG RUNNER_BASE_IMAGE
FROM ${RUNNER_BASE_IMAGE}
ARG RUNNER_BASE_IMAGE

USER root
RUN test -x /usr/bin/python3 \
    && /usr/bin/python3 -I -c 'import os,re; assert re.fullmatch(r"ghcr\.io/actions/actions-runner(?::[A-Za-z0-9_.-]+)?@sha256:[0-9a-f]{64}", os.environ["RUNNER_BASE_IMAGE"]), "RUNNER_BASE_IMAGE must pin an official Runner image digest"' \
    && test -x /home/runner/bin/Runner.Listener \
    && install -d -o runner -g "$(id -gn runner)" -m 0700 /shaula

COPY --chown=root:root --chmod=0555 bootstrap-shim /usr/local/bin/bootstrap-shim

WORKDIR /home/runner
USER runner
ENTRYPOINT []
CMD ["/usr/local/bin/bootstrap-shim"]
