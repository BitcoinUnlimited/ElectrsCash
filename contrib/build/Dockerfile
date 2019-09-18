# Dockerfile that creates a container image that continuosuly builds the
# latest commit of ElectrsCash and publishes it on a webserver at port 8000.
#
# To build & run this docker image:
# $ docker build -t electrscash/publisher .
# $ docker run -p 8000:8000 electrscash/publisher
FROM rust:buster
RUN apt-get update
RUN apt-get install -y clang cmake git python3 python3-git
RUN mkdir /tools
COPY latest-build-publisher.py /tools
COPY utilbuild.py /tools
CMD ["/tools/latest-build-publisher.py", "--verbose"]
