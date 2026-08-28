# Security policy

tapkey handles API credentials and edits configuration files that other tools authenticate with, so
a defect here can send somebody's key somewhere they did not choose. Reports are welcome and taken
seriously.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository:
**Security → Report a vulnerability**. It opens a private thread visible only to the maintainers.

Please do not open a public issue for a security defect until it has been fixed and released.

Useful things to include, as far as you have them: what you did, what you observed, which version or
commit, which operating system, and which of the managed tools was involved. If a credential of your
own was exposed while you were investigating, rotate it and say so — do not include its value in the
report.

## Scope

In scope: anything that causes a credential to be written where it should not be, sent to an endpoint
the user did not select, or exposed in a log, an error message or a crash report. Also in scope: any
path that corrupts or destroys a managed configuration file, or that reports an effective state which
does not match what the tool would actually use.

Out of scope: vulnerabilities in the coding tools tapkey manages, or in the providers it points them
at. Please report those to their maintainers. We are glad to hear about them anyway if tapkey makes
them easier to hit.

## What to expect

An acknowledgement within a few days, and an assessment of whether we can reproduce it. Since there
is no released build yet, there is no fixed disclosure timetable; once there is one, this section
will name it.
