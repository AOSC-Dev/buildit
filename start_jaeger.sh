#!/bin/sh
# start jaeger for opentelemetry tracing
mkdir jaeger
sudo chown 10001 jaeger
sudo docker run \
    -e SPAN_STORAGE_TYPE=badger \
    -e BADGER_EPHEMERAL=false \
    -e BADGER_DIRECTORY_VALUE=/badger/data \
    -e BADGER_DIRECTORY_KEY=/badger/key \
    -v $PWD/jaeger:/badger \
    -p 127.0.0.1:4318:4318 \
    -p 127.0.0.1:16686:16686 \
    --name jaeger \
    --rm \
    jaegertracing/all-in-one:1.55