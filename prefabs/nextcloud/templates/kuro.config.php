<?php
// Écrit par Kuro à chaque démarrage : ne pas modifier, utiliser config.php.
$CONFIG = array (
  // Site servi en HTTPS par le proxy du nœud, sur le réseau privé du serveur.
  'trusted_domains' => array (0 => '{{ server.web_host }}'),
  'overwrite.cli.url' => '{{ server.web_url }}',
  'overwriteprotocol' => 'https',
  'trusted_proxies' => array ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'),
  // Valeurs des fichiers par défaut de l'image (apps, apcu, upgrade-disable-web).
  'apps_paths' => array (
    0 => array ('path' => OC::$SERVERROOT . '/apps', 'url' => '/apps', 'writable' => false),
    1 => array ('path' => OC::$SERVERROOT . '/custom_apps', 'url' => '/custom_apps', 'writable' => true),
  ),
  'memcache.local' => '\\OC\\Memcache\\APCu',
  'upgrade.disable-web' => true,
);
