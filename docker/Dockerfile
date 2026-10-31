# syntax=docker/dockerfile:1
# Image officielle de l'API Kuro (ADR-0016, amendement) : le binaire musl de la release, le panel
# et les prefabs officiels, aux mêmes chemins qu'install.sh. Construite par release.yml à partir
# des artefacts déjà compilés ; le contexte est un dossier `image/` qui contient
# `linux/<amd64|arm64>/kuro-api`, `panel/`, `prefabs/` et `entrypoint.sh`.
#
# Le daemon (rôle node) n'est pas dans l'image : il pilote le Docker de l'hôte.

FROM docker.io/library/debian:bookworm-slim

ARG TARGETARCH

# ca-certificates : sources de contenu et webhooks en HTTPS ; curl : sonde de santé ;
# tini : kuro-api reçoit SIGTERM même en PID 1.
RUN apt-get update -qq \
    && apt-get install -y -qq --no-install-recommends ca-certificates curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 1000 kuro \
    && useradd --system --uid 1000 --gid kuro --home-dir /var/lib/kuro --shell /usr/sbin/nologin kuro \
    && install -d -m 0750 -o kuro -g kuro /var/lib/kuro /etc/kuro

COPY --chmod=0755 linux/${TARGETARCH}/kuro-api /usr/local/bin/kuro-api
COPY --chmod=0755 entrypoint.sh /usr/local/bin/kuro-entrypoint
COPY panel /usr/share/kuro/panel
COPY prefabs /usr/share/kuro/prefabs

# Configuration par variables `KURO_API_*` (sections imbriquées avec `__`). Un fichier TOML monté
# peut la compléter : KURO_API_CONFIG=/etc/kuro/api.toml. La PKI des nœuds vit dans le volume.
ENV KURO_API_HTTP__BIND=0.0.0.0:8080 \
    KURO_API_HTTP__PANEL_DIR=/usr/share/kuro/panel \
    KURO_API_PREFABS__DIR=/usr/share/kuro/prefabs \
    KURO_API_DAEMON__BIND=0.0.0.0:8443 \
    KURO_API_DAEMON__CA_CERT=/var/lib/kuro/pki/ca.pem \
    KURO_API_DAEMON__CA_KEY=/var/lib/kuro/pki/ca.key \
    KURO_API_DAEMON__CERT=/var/lib/kuro/pki/api.pem \
    KURO_API_DAEMON__KEY=/var/lib/kuro/pki/api.key \
    KURO_API_LOG__FORMAT=json

USER kuro
WORKDIR /var/lib/kuro
VOLUME /var/lib/kuro
# 8080 : panel et API, derrière un reverse proxy HTTPS ; 8443 : nœuds (mTLS).
EXPOSE 8080 8443

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

ENTRYPOINT ["tini", "--", "kuro-entrypoint"]
CMD ["serve"]
