# Buildit

## Frontend

The current frontend is Telegram bot @aosc_buildit_bot.

## Backend Server

Backend server handles new requests from Frontend. It sends new jobs to message queue.

If the request contains multiple architectures, it is split into multiple ones for each architecture.

## Message Queue

Use RabbitMQ:

```shell
docker run -it --rm --name rabbitmq -p 5672:5672 -p 15672:15672 rabbitmq:3.12-management
# alternatively
cd rabbitmq
docker compose up -d
```

Queues:

1. Job submission: job-[arch]
2. Job completion: job-completion

## Worker Server

Worker server fetches jobs from message queue and runs ciel to do the actual build. The result is reported to backend server via message queue.

Each worker can only build for one architecture.

## Job

Each job contains the following arguments:

1. One or more packages to build
2. Git ref of aosc-os-abbs repo
3. Architecture e.g. amd64 to filter worker

Job result:

1. List of successful builds
2. Failed package and link to build log (on paste.aosc.io)