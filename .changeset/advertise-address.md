---
default: patch
---

# Servers advertise the address of `network.interface`

With `network.interface`, a server now also writes `advertise-address`, the interface's address in the family of the service network: IPv6 when `service-cidr` starts with an IPv6 range or the interface has no IPv4 address, else IPv4. Before, the API server advertised its public address, and pods that reached the `kubernetes` Service timed out when 6443 was closed on the public interface.
