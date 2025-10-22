# Security Policy

## Supported Versions

We actively support and provide security updates for the following versions:

| Version | Supported          |
| ------- | ------------------ |
| 1.x.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

We take security seriously. If you discover a security vulnerability in rustvncserver, please report it responsibly.

### How to Report

**Please DO NOT create a public GitHub issue for security vulnerabilities.**

Instead, please report security issues via email to:

**dustin.mcafee@my.maryvillecollege.edu**

### What to Include

Please include the following information in your report:

- Description of the vulnerability
- Steps to reproduce the issue
- Potential impact
- Suggested fix (if you have one)
- Your contact information

### Response Timeline

- **Initial Response**: Within 48 hours
- **Status Update**: Within 7 days
- **Fix Timeline**: Varies by severity (critical issues prioritized)

### Disclosure Policy

- We will acknowledge your report within 48 hours
- We will provide regular updates on our progress
- We will notify you when the vulnerability is fixed
- We will publicly disclose the vulnerability after a fix is released
- We will credit you in the security advisory (unless you prefer to remain anonymous)

### Security Best Practices

When using rustvncserver in production:

1. **Use Strong Passwords**: Always set a strong VNC password
2. **Use Encryption**: Consider using VNC over SSH tunnel or VPN
3. **Network Security**: Restrict VNC port access with firewall rules
4. **Keep Updated**: Always use the latest version with security patches
5. **Monitor Logs**: Watch for suspicious connection attempts
6. **Limited Access**: Only allow connections from trusted IP addresses

### Known Security Considerations

- **VNC Authentication**: The standard VNC authentication (DES-based) is not cryptographically strong by modern standards. For sensitive environments, always use VNC over an encrypted tunnel (SSH, VPN, TLS).
- **No Encryption**: The RFB protocol itself does not provide encryption. All data, including the framebuffer, is sent in plaintext after authentication.
- **Brute Force**: VNC authentication is vulnerable to brute-force attacks. Use strong passwords and consider rate limiting at the network level.

### Recommended Deployment

For production environments, we recommend:

```bash
# Example: VNC over SSH tunnel
ssh -L 5900:localhost:5900 user@vnc-server
# Then connect to localhost:5900 from your VNC client
```

## Security Updates

Security fixes will be released as:
- Patch releases for supported versions (1.x.y → 1.x.z)
- Announced in CHANGELOG.md
- Tagged as security releases in GitHub

Subscribe to repository releases to be notified of security updates.

## Hall of Fame

We recognize and thank security researchers who responsibly disclose vulnerabilities:

(No reports yet - be the first!)

---

Thank you for helping keep rustvncserver and its users safe!
