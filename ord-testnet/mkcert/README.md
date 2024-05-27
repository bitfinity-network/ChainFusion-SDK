# Linux install

In order to make the certificate working on localhost on Linux systems, you have to install the certificates.

```sh
sudo cp rootCA.pem /etc/ca-certificates/trust-source/anchors/ckerc20.pem
sudo cp localhost+3.pem /etc/ca-certificates/trust-source/anchors/ckerc20-local.pem
sudo update-ca-trust
```
