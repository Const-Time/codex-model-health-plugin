FROM python:3.12-slim-bookworm@sha256:54c85f3c47607a77f32adec749d3c81d1348bf25833671f512b26a9b6d778cb3 AS build
RUN sed -i 's|http://deb.debian.org|https://deb.debian.org|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends binutils && rm -rf /var/lib/apt/lists/*
COPY engine/requirements-build.txt /tmp/requirements-build.txt
RUN pip install --no-cache-dir -r /tmp/requirements-build.txt
WORKDIR /src
COPY vendor/meow /src/vendor/meow
COPY vendor/modeltrace /src/vendor/modeltrace
COPY engine/worker.py engine/modeltrace_adapter.py /src/engine/
RUN pyinstaller --clean --noconfirm --onefile --name meow-engine \
    --paths /src/vendor/meow \
    --paths /src/vendor/modeltrace \
    --add-data /src/vendor/modeltrace/data:vendor/modeltrace/data \
    --add-data /src/vendor/modeltrace/SOURCE.json:vendor/modeltrace \
    --add-data /src/vendor/meow/gpt56_vnext/baselines:vendor/meow/gpt56_vnext/baselines \
    engine/worker.py
COPY engine/collect_licenses.py /src/engine/collect_licenses.py
RUN python /src/engine/collect_licenses.py /src/dist/ENGINE-LICENSES.txt && pip freeze > /src/dist/ENGINE-REQUIREMENTS.txt
FROM scratch AS artifact
COPY --from=build /src/dist/meow-engine /meow-engine
COPY --from=build /src/dist/ENGINE-LICENSES.txt /ENGINE-LICENSES.txt
COPY --from=build /src/dist/ENGINE-REQUIREMENTS.txt /ENGINE-REQUIREMENTS.txt
