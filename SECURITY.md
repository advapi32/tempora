# Reporting a vulnerability

Use the GitHub repository's **Security** tab, open **Advisories**, and choose
**Report a vulnerability** to contact the maintainers privately. Include the app
and Windows versions, the affected component, and enough detail to reproduce the
issue with sample data. Keep recorded history and personal paths out of the
report. [GitHub private vulnerability reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/report-privately)

## Recorder control pipe

The local named pipe grants access only to the current Windows logon and
rejects remote clients. The recorder also validates the client's Windows session.
Before sending a request, the client checks that the server process belongs to
the same logon and session.
