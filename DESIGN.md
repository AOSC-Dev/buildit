# BuildIt! 2

## Overview

BuildIt! 2 is the latest generation of BuildIt! build automation system for AOSC OS.

## Frontend

BuildIt! 2 have the following frontends:

1. Web: hosted at https://buildit.aosc.io
2. Telegram: Telegram bot @aosc_buildit_bot
3. GitHub: add @aosc-buildit-bot in pr comments

## Database

BuildIt! 2 uses PostgreSQL as the database.

The database should have the following tables:

1. pipelines: pipeline is a series of jobs
2. jobs: job is a specific task for worker to do
3. packages: track the status of packages in stable branch
4. users: track github and telegram user association

The terms `pipelines` and `jobs` are taken from GitLab CI.

## Backend

Backend server handles new requests from Frontend and create new pipeline. The pipeline may contain multi architectures, then it is split into multiple jobs for each architecture.

Worker can grab new job from backend server. Workers need to send heartbeat to the server periodically reporting its current state, otherwise the job will be rescheduled.

## Worker

Worker server fetches jobs from backend and runs ciel to do the actual build. The result is reported to backend server via HTTP api endpoint.

Each worker can only build for one architecture.

## Job

Each job contains the following arguments:

1. One or more packages to build
2. Git ref of aosc-os-abbs repo
3. Architecture e.g. amd64 to filter worker

Job result:

1. List of successful builds
2. Failed package and link to build log (on buildit.aosc.io)

Job status:

1. created: can be assigned to worker
2. running: assigned to worker
3. error: unexpected error
4. success: finished, build_success && pushpkg_success
5. failed: finished, !build_success || !pushpkg_success

Pipeline status is computed from job status:

1. error: any job has status `error`
2. failed: any job has status `failed`, no job has status `error`
3. success: all job has status `success`
4. running: otherwise

## Authentication

Authentication:

1. Web: login via GitHub App
2. Telegram: jump to GitHub App and authenticate, associate with Telegram user
3. GitHub: username provided by GitHub

User roles:

1. Anonymous: not logged-in
2. Guest: logged-in, but not in AOSC-Dev GitHub organization
3. Developer: loggined and in AOSC-Dev GitHub organization
