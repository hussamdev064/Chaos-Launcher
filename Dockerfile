# syntax=docker/dockerfile:1
#
# ChaosLauncher build image. This is built locally on first launch by the Rust
# backend; the launcher never pulls a prebuilt game binary or game assets.
# Each stage installs one toolchain family; the final stage combines them so a
# single image can serve Linux, Windows, and Android targets.

# ---------------------------------------------------------------------------
# Stage 1: Android SDK + NDK (large downloads kept out of the final layer set).
# ---------------------------------------------------------------------------
FROM ubuntu:22.04 AS android-toolchain

ARG ANDROID_NDK_VERSION=r25c
ARG ANDROID_CMDLINE_TOOLS_REVISION=11076708
ARG ANDROID_PLATFORM=android-34
ARG ANDROID_BUILD_TOOLS=34.0.0
ENV DEBIAN_FRONTEND=noninteractive
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates unzip wget openjdk-17-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

RUN wget -q "https://dl.google.com/android/repository/android-ndk-${ANDROID_NDK_VERSION}-linux.zip" -O /tmp/ndk.zip \
    && unzip -q /tmp/ndk.zip -d /opt \
    && mv "/opt/android-ndk-${ANDROID_NDK_VERSION}" /opt/android-ndk \
    && rm /tmp/ndk.zip

RUN wget -q "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS_REVISION}_latest.zip" -O /tmp/cmdline.zip \
    && mkdir -p /opt/android-sdk/cmdline-tools \
    && unzip -q /tmp/cmdline.zip -d /opt/android-sdk/cmdline-tools \
    && mv /opt/android-sdk/cmdline-tools/cmdline-tools /opt/android-sdk/cmdline-tools/latest \
    && rm /tmp/cmdline.zip

ENV ANDROID_HOME=/opt/android-sdk
ENV ANDROID_SDK_ROOT=/opt/android-sdk
RUN yes | /opt/android-sdk/cmdline-tools/latest/bin/sdkmanager --sdk_root="${ANDROID_SDK_ROOT}" --licenses >/dev/null 2>&1 || true \
    && /opt/android-sdk/cmdline-tools/latest/bin/sdkmanager --sdk_root="${ANDROID_SDK_ROOT}" \
       "platform-tools" "platforms;${ANDROID_PLATFORM}" "build-tools;${ANDROID_BUILD_TOOLS}"

# ---------------------------------------------------------------------------
# Stage 2: appimagetool binary, copied into the final image below.
# ---------------------------------------------------------------------------
FROM ubuntu:22.04 AS appimage-toolchain

ARG APPIMAGETOOL_URL=https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/*

RUN wget -q "${APPIMAGETOOL_URL}" -O /usr/local/bin/appimagetool \
    && chmod +x /usr/local/bin/appimagetool

# ---------------------------------------------------------------------------
# Final stage: distro packages + copied SDKs. WORKDIR is the build mount point
# used by Phase 4 (`/work` holds cloned source and the user's Data.rsdk).
# ---------------------------------------------------------------------------
FROM ubuntu:22.04 AS builder

ARG DEBIAN_FRONTEND=noninteractive
ARG GRADLE_VERSION=8.6
ARG OGG_VERSION=1.3.5
ARG THEORA_VERSION=1.1.1

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build git wget ca-certificates unzip zip pkg-config python3 file \
        libsdl2-dev libglew-dev libogg-dev libvorbis-dev libtheora-dev \
        mingw-w64 gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 \
        openjdk-17-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

# Upstream uses Windows-style header casing; MinGW ships lowercase filenames.
RUN for pair in \
        'xaudio2.h:XAudio2.h' \
        'xaudio2fx.h:XAudio2fx.h' \
        'xinput.h:Xinput.h' \
        'xinput.h:XInput.h' \
        'windows.h:Windows.h' \
        'd3dcompiler.h:D3Dcompiler.h' \
        'mmsystem.h:timeapi.h' \
        'ks.h:Ks.h' \
        'ksmedia.h:Ksmedia.h'; do \
        lower=${pair%%:*}; upper=${pair##*:}; \
        ln -sf "$lower" "/usr/x86_64-w64-mingw32/include/$upper"; \
    done

# Upstream links -lXInput; MinGW installs the import library as libxinput.a.
RUN ln -sf libxinput.a /usr/x86_64-w64-mingw32/lib/libXInput.a

# Windows cross-builds need MinGW static libs. The pinned engine checkout omits
# its bundled Ogg/Theora fallback sources, so build them from upstream releases.
RUN set -eu; \
    cd /tmp; \
    wget -q "https://downloads.xiph.org/releases/ogg/libogg-${OGG_VERSION}.tar.gz"; \
    sha256sum "libogg-${OGG_VERSION}.tar.gz"; \
    tar -xzf "libogg-${OGG_VERSION}.tar.gz"; \
    cd "/tmp/libogg-${OGG_VERSION}"; \
    ./configure --host=x86_64-w64-mingw32 --prefix=/usr/x86_64-w64-mingw32 --disable-shared --enable-static; \
    make -j"$(nproc)"; \
    make install; \
    cd /tmp; \
    wget -q "https://downloads.xiph.org/releases/theora/libtheora-${THEORA_VERSION}.tar.gz"; \
    sha256sum "libtheora-${THEORA_VERSION}.tar.gz"; \
    tar -xzf "libtheora-${THEORA_VERSION}.tar.gz"; \
    cd "/tmp/libtheora-${THEORA_VERSION}"; \
    export PKG_CONFIG_PATH=/usr/x86_64-w64-mingw32/lib/pkgconfig; \
    export PKG_CONFIG_LIBDIR=/usr/x86_64-w64-mingw32/lib/pkgconfig; \
    ./configure --host=x86_64-w64-mingw32 --prefix=/usr/x86_64-w64-mingw32 --disable-shared --enable-static --disable-examples; \
    make -j"$(nproc)"; \
    make install; \
    cd /tmp; \
    rm -rf "/tmp/libogg-${OGG_VERSION}" "/tmp/libogg-${OGG_VERSION}.tar.gz" \
           "/tmp/libtheora-${THEORA_VERSION}" "/tmp/libtheora-${THEORA_VERSION}.tar.gz"

# CONFIG-mode package files let upstream Windows.cmake find the static libs it
# expects as Ogg::ogg and Theora::theora.
RUN mkdir -p /usr/x86_64-w64-mingw32/lib/cmake/Ogg /usr/x86_64-w64-mingw32/lib/cmake/Theora \
    && printf '%s\n' \
        'if(NOT TARGET Ogg::ogg)' \
        '  add_library(Ogg::ogg STATIC IMPORTED)' \
        '  set_target_properties(Ogg::ogg PROPERTIES' \
        '    IMPORTED_LOCATION "/usr/x86_64-w64-mingw32/lib/libogg.a"' \
        '    INTERFACE_INCLUDE_DIRECTORIES "/usr/x86_64-w64-mingw32/include"' \
        '  )' \
        'endif()' \
        > /usr/x86_64-w64-mingw32/lib/cmake/Ogg/OggConfig.cmake \
    && printf '%s\n' \
        'find_package(Ogg CONFIG REQUIRED)' \
        'if(NOT TARGET Theora::theora)' \
        '  add_library(Theora::theora STATIC IMPORTED)' \
        '  set_target_properties(Theora::theora PROPERTIES' \
        '    IMPORTED_LOCATION "/usr/x86_64-w64-mingw32/lib/libtheora.a"' \
        '    INTERFACE_INCLUDE_DIRECTORIES "/usr/x86_64-w64-mingw32/include"' \
        '    INTERFACE_LINK_LIBRARIES "Ogg::ogg;m"' \
        '  )' \
        'endif()' \
        > /usr/x86_64-w64-mingw32/lib/cmake/Theora/TheoraConfig.cmake

RUN wget -q "https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip" -O /tmp/gradle.zip \
    && unzip -q /tmp/gradle.zip -d /opt \
    && ln -s "/opt/gradle-${GRADLE_VERSION}" /opt/gradle \
    && rm /tmp/gradle.zip

COPY --from=android-toolchain /opt/android-ndk /opt/android-ndk
COPY --from=android-toolchain /opt/android-sdk /opt/android-sdk
COPY --from=appimage-toolchain /usr/local/bin/appimagetool /usr/local/bin/appimagetool
COPY docker/mingw-x86_64.cmake /opt/cmake/mingw-x86_64.cmake

# The pinned engine expects libtheora sources under dependencies/android but
# omits them. Vendoring the already-approved upstream source keeps Android
# builds self-contained without fetching game assets or prebuilt binaries.
COPY vendor/libtheora-1.1.1.tar.gz /opt/android-deps/libtheora-1.1.1.tar.gz
RUN mkdir -p /opt/android-deps/libtheora \
    && tar -xzf /opt/android-deps/libtheora-1.1.1.tar.gz \
       -C /opt/android-deps/libtheora --strip-components=1 \
    && rm /opt/android-deps/libtheora-1.1.1.tar.gz

COPY vendor/libogg-1.3.5.tar.gz /opt/android-deps/libogg-1.3.5.tar.gz
RUN mkdir -p /opt/android-deps/libogg \
    && tar -xzf /opt/android-deps/libogg-1.3.5.tar.gz \
       -C /opt/android-deps/libogg --strip-components=1 \
    && rm /opt/android-deps/libogg-1.3.5.tar.gz

ENV ANDROID_HOME=/opt/android-sdk
ENV ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_NDK_HOME=/opt/android-ndk
ENV PATH="/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/opt/gradle/bin:${PATH}"

# appimagetool is an AppImage; without FUSE in Docker, Phase 4 should invoke it as
# `appimagetool --appimage-extract-and-run ...`.
WORKDIR /work
