#!/usr/bin/env bash
# Creates the self-signed code signing certificate that keeps fastty's macOS
# identity stable across builds.
#
# An ad-hoc signature embeds the content hash in the app's designated
# requirement. The hash changes on every build, so macOS treats every update as
# a different app and asks the user to accept it again. A self-signed
# certificate with a pinned leaf hash produces the same requirement every time,
# so the record survives updates.
#
# Run this once. It prints the line to paste into
# assets/macos-signing-requirement.txt.

set -euo pipefail

CERT_NAME="${FASTTY_CERT_NAME:-fastty-dev}"
BUNDLE_ID="com.diegoleteliers10.fastty"
P12_PASSWORD="${FASTTY_CERT_PASSWORD:-fasttydev}"

if security find-certificate -c "$CERT_NAME" >/dev/null 2>&1; then
    echo "Certificate '$CERT_NAME' already exists in the login keychain."
else
    echo "Creating self-signed code signing certificate: $CERT_NAME"
    CONFIG_FILE=$(mktemp /tmp/fastty-cert.XXXXXX.cnf)
    KEY_FILE=$(mktemp /tmp/fastty-key.XXXXXX.pem)
    CERT_FILE=$(mktemp /tmp/fastty-cert.XXXXXX.crt)
    P12_FILE=$(mktemp /tmp/fastty-cert.XXXXXX.p12)
    trap 'rm -f "$CONFIG_FILE" "$KEY_FILE" "$CERT_FILE" "$P12_FILE"' EXIT

    cat <<EOF > "$CONFIG_FILE"
[ req ]
default_bits        = 2048
distinguished_name  = req_distinguished_name
prompt              = no
x509_extensions     = v3_ca

[ req_distinguished_name ]
CN                  = $CERT_NAME

[ v3_ca ]
keyUsage            = critical, digitalSignature
extendedKeyUsage    = critical, codeSigning
EOF

    /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
        -keyout "$KEY_FILE" -out "$CERT_FILE" \
        -config "$CONFIG_FILE"

    /usr/bin/openssl pkcs12 -export -inkey "$KEY_FILE" -in "$CERT_FILE" \
        -out "$P12_FILE" -passout "pass:$P12_PASSWORD" \
        -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1

    # A self-signed certificate is not trusted by default, and codesign refuses
    # to use an untrusted identity: `find-identity` reports nothing and every
    # later codesign call fails with "no identity found". Trust it as a root and
    # mark it for code signing before importing.
    security add-trusted-cert -d -r trustRoot \
        -k ~/Library/Keychains/login.keychain-db "$CERT_FILE"
    security add-trusted-cert -d -p codeSign \
        -k ~/Library/Keychains/login.keychain-db "$CERT_FILE"

    security import "$P12_FILE" -k ~/Library/Keychains/login.keychain-db \
        -f pkcs12 -P "$P12_PASSWORD" -T /usr/bin/codesign

    if ! security find-identity -p codesigning | grep -qF "$CERT_NAME"; then
        echo "The certificate did not import as a usable code signing identity."
        echo "Check with: security find-identity -p codesigning"
        exit 1
    fi

    echo "Certificate imported and trusted."
fi

# Sign a scratch bundle with the new identity and read back the requirement
# macOS will record for it.
WORK=$(mktemp -d /tmp/fastty-sign-probe.XXXXXX)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/Probe.app/Contents/MacOS"
cp /bin/echo "$WORK/Probe.app/Contents/MacOS/Probe"
cat > "$WORK/Probe.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>Probe</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
EOF

if ! codesign --force --deep -s "$CERT_NAME" "$WORK/Probe.app"; then
    echo "codesign could not use the identity '$CERT_NAME'."
    echo "Check with: security find-identity -p codesigning"
    exit 1
fi
# `codesign -dr -` writes the requirement to stderr, prefixed with "# ".
ACTUAL=$(codesign -dr - "$WORK/Probe.app" 2>&1 | sed -n 's/^#[[:space:]]*designated => //p')

if [ -z "$ACTUAL" ]; then
    echo "Could not read the designated requirement. Check that codesign can see the identity:"
    echo "  security find-identity -p codesigning"
    exit 1
fi

echo
echo "Paste this line into assets/macos-signing-requirement.txt:"
echo
echo "$ACTUAL"
echo
echo "Then add these GitHub Actions secrets:"
echo "  FASTTY_CERTIFICATE_P12_BASE64  base64 < the exported .p12"
echo "  FASTTY_CERTIFICATE_PASSWORD    the .p12 password"
echo "  FASTTY_SIGNING_IDENTITY        $CERT_NAME"
