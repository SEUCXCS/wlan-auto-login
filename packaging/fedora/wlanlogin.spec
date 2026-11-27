# Template for `cargo xtask bundle rpm`, which fills the @NAME@ placeholders
# and stages the built CLI and documents in the SOURCES directory.
Name:           @APP_NAME@
Version:        @VERSION@
Release:        @RELEASE@%{?dist}
Summary:        Automatic login for the Student_CX campus Wi-Fi portal
License:        MIT
URL:            https://github.com/SEUCXCS/wlan-auto-login
Requires:       NetworkManager
Requires:       systemd
Suggests:       gnome-keyring

# The release profile already strips the prebuilt binary; there is nothing to split out,
# and build-id links would differ between rpmbuild hosts.
%global debug_package %{nil}
%global _build_id_links none

%description
wlanlogin signs in to the Student_CX captive portal and keeps the session
online from a systemd service. User-scope credentials are kept in the
Secret Service; system-scope credentials use systemd-creds.

The package installs only the command. Run "wlanlogin service install"
to save credentials and register the background service.

%install
install -Dpm 0755 %{_sourcedir}/%{name} -t %{buildroot}%{_bindir}
install -Dpm 0644 %{_sourcedir}/LICENSE -t %{buildroot}%{_licensedir}/%{name}
install -Dpm 0644 %{_sourcedir}/README.md %{_sourcedir}/CHANGELOG.md -t %{buildroot}%{_docdir}/%{name}
install -Dpm 0644 %{_sourcedir}/%{name}.1 -t %{buildroot}%{_mandir}/man1
install -Dpm 0644 %{_sourcedir}/%{name}.zh_CN.1 %{buildroot}%{_mandir}/zh_CN/man1/%{name}.1

%files
%dir %{_licensedir}/%{name}
%license %{_licensedir}/%{name}/LICENSE
%dir %{_docdir}/%{name}
%doc %{_docdir}/%{name}/README.md
%doc %{_docdir}/%{name}/CHANGELOG.md
%{_bindir}/%{name}
%{_mandir}/man1/%{name}.1*
%lang(zh_CN) %{_mandir}/zh_CN/man1/%{name}.1*

%changelog
* @DATE@ Goldbro233 <bowensun_06@outlook.com> - @VERSION@-@RELEASE@
- Package upstream release @UPSTREAM@; see CHANGELOG.md.
