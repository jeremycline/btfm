FROM registry.fedoraproject.org/fedora:36 as builder

USER root
RUN dnf install -y \
	https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-36.noarch.rpm \
	https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-36.noarch.rpm && \
	dnf install -y \
	autoconf \
    curl \
	cmake \
    ffmpeg-devel \
	glib2-devel \
	gcc-g++ \
	gstreamer1-devel \
	gstreamer1-plugins-base-devel \
	gstreamer1-plugins-bad-free-devel \
	libsodium-devel \
	libstdc++-devel \
	make \
	openssl-devel \
	opus-devel \
    python3-devel \
    pipx

RUN curl https://sh.rustup.rs -sSf | sh -s -- --profile minimal --default-toolchain stable -y
RUN pipx install "maturin>=0.14,<0.15" && pipx install "patchelf"
WORKDIR /builddir
COPY . ./
RUN source "$HOME/.cargo/env" && \
    cd btfm && \
	RUSTFLAGS="-g -C link-arg=-Wl,--compress-debug-sections=zlib -C force-frame-pointers=yes" \
    PATH=$PATH:/root/.local/bin maturin build

FROM registry.fedoraproject.org/fedora:36
USER root
RUN dnf install -y \
	https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-36.noarch.rpm \
	https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-36.noarch.rpm && \
	dnf install -y \
	ffmpeg \
	glib2 \
	gstreamer1 \
	gstreamer1-plugins-good \
	gstreamer1-plugins-good-extras \
	gstreamer1-plugins-bad-free \
	gstreamer1-plugins-ugly \
	gstreamer1-plugins-ugly-free \
	gstreamer1-plugin-libav \
	libsodium \
	openssl \
	opus \
    pipx
COPY --from=builder /builddir/target/wheels/btfm_server* /tmp/

RUN useradd --create-home btfm && \
    mkdir -p /var/lib/btfm && \
    chown btfm:btfm -R /var/lib/btfm
USER btfm
RUN pipx install /tmp/*.whl
ENTRYPOINT ["/home/btfm/.local/bin/btfm-server"]
