FROM registry.fedoraproject.org/fedora:37

RUN dnf install -y https://download1.rpmfusion.org/free/fedora/rpmfusion-free-release-37.noarch.rpm
RUN dnf install -y \
	autoconf \
	cargo \
	clippy \
	diffutils \
	ffmpeg \
	file \
	findutils \
	libsodium-devel \
	make \
	openssl-devel \
	opus-devel \
	rustfmt \
	clang \
	glib2-devel \
	gcc-g++ \
	libstdc++-devel \
	gstreamer1-devel \
	gstreamer1-plugins-base-devel

RUN mkdir /root/whisper
RUN curl -L https://github.com/mozilla/DeepSpeech/releases/download/v0.7.0/native_client.amd64.cpu.linux.tar.xz -o /root/whisper/
RUN cd /root/deep_speech && tar xvf deep_speech.tar.xz
RUN mv /root/deep_speech/libdeepspeech.so /usr/lib64/

# This way we can cache all the build dependencies
RUN cd /devel && cargo build
# This way we can cache all the development dependencies
RUN cd /devel && cargo test
# This way we can cache all the doc dependencies
RUN cd /devel && cargo doc
RUN cargo install cargo-audit

CMD ["bash"]
