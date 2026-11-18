#!/bin/sh
# Exemple de branchement de collaboration « Ma propre commande ».
# Dans l'application : Collaboration → Ma propre commande →
#   sh /chemin/vers/connecteur-exemple.sh
#
# L'application appelle ce script avec « pull » ou « push » :
#   pull : afficher sur la sortie l'espace (JSON) des autres, ou rien
#   push : recevoir l'espace (JSON) sur l'entrée et le publier
#
# Ici on publie simplement sur un serveur web à vous (à remplacer) :
#   GET  $SERVEUR/espace.json  → la copie commune
#   PUT  $SERVEUR/espace.json  ← notre copie
# Remplacez ces deux lignes par ce que vous voulez : base de données,
# dépôt git, bot de messagerie, etc.

SERVEUR="${ORCHESTRA_SERVEUR:-https://exemple.invalid/notes}"

case "$1" in
  pull) curl -fsS "$SERVEUR/espace.json" 2>/dev/null || true ;;
  push) curl -fsS -X PUT -H 'Content-Type: application/json' --data-binary @- "$SERVEUR/espace.json" ;;
  *) echo "usage: $0 pull|push" >&2; exit 2 ;;
esac
