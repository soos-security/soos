Name: soos
Version: 0.1.0
Release: 1%{?dist}
Summary: Local facial biometric PAM module and daemon for Linux

License:        Apache-2.0 OR MIT
URL:            https://github.com/Mysticaly622/soos
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  pam-devel
BuildRequires:  clang-devel
BuildRequires:  gcc
BuildRequires:  make
BuildRequires:  systemd-rpm-macros

Requires:       pam
Requires:       shadow-utils
Requires:       systemd

%description
soos is a zero-trust local facial biometric PAM subsystem for Linux.
It provides sub-250ms verification latency, warm camera streaming via V4L2,
presentation attack detection (PAD), and encrypted vector storage at rest.

%prep
%setup -q

%build
cargo build --release --workspace

%install
rm -rf %{buildroot}

# Install binaries
install -d -m 0755 %{buildroot}/usr/libexec/soos
install -m 0755 target/release/soos-daemon %{buildroot}/usr/libexec/soos/soos-daemon

install -d -m 0755 %{buildroot}%{_bindir}
install -m 0755 target/release/soos-admin %{buildroot}%{_bindir}/soos-admin
install -m 0755 target/release/soos-enroll %{buildroot}%{_bindir}/soos-enroll

# Install PAM module
install -d -m 0755 %{buildroot}%{_libdir}/security
install -m 0644 target/release/libpam_soos.so %{buildroot}%{_libdir}/security/pam_soos.so

# Install systemd service unit
install -d -m 0755 %{buildroot}%{_unitdir}
install -m 0644 packaging/soos-daemon.service %{buildroot}%{_unitdir}/soos-daemon.service

# Install Fedora custom authselect profile
install -d -m 0755 %{buildroot}%{_sysconfdir}/authselect/custom/soos
cp -r packaging/pam/fedora/soos/* %{buildroot}%{_sysconfdir}/authselect/custom/soos/

# Provision state and runtime directories
install -d -m 0755 %{buildroot}%{_sharedstatedir}/soos
install -d -m 0700 %{buildroot}%{_sharedstatedir}/soos/biometrics
install -d -m 0700 %{buildroot}%{_sharedstatedir}/soos/evidence
install -d -m 0755 %{buildroot}%{_sharedstatedir}/soos/models
install -d -m 0750 %{buildroot}%{_rundir}/soos

%pre
# Create system group 'soos' if absent
if ! getent group soos >/dev/null 2>&1; then
    groupadd -r soos || :
fi

%post
%systemd_post soos-daemon.service

# Provision directory hierarchy and permissions
chmod 0755 %{_sharedstatedir}/soos || :
chmod 0700 %{_sharedstatedir}/soos/biometrics || :
chmod 0700 %{_sharedstatedir}/soos/evidence || :
chmod 0755 %{_sharedstatedir}/soos/models || :

# Generate cryptographic master key (32 bytes, mode 0600) if absent
if [ ! -f "%{_sharedstatedir}/soos/master.key" ]; then
    if command -v openssl >/dev/null 2>&1; then
        openssl rand 32 > "%{_sharedstatedir}/soos/master.key" 2>/dev/null || :
    else
        head -c 32 /dev/urandom > "%{_sharedstatedir}/soos/master.key" 2>/dev/null || :
    fi
    chmod 0600 "%{_sharedstatedir}/soos/master.key" || :
fi

# Ensure runtime socket directory exists with proper permissions
mkdir -p %{_rundir}/soos
chmod 0750 %{_rundir}/soos || :
chown root:soos %{_rundir}/soos 2>/dev/null || :

%preun
%systemd_preun soos-daemon.service

%postun
%systemd_postun_with_restart soos-daemon.service

%files
/usr/libexec/soos/soos-daemon
%{_bindir}/soos-admin
%{_bindir}/soos-enroll
%{_libdir}/security/pam_soos.so
%{_unitdir}/soos-daemon.service
%{_sysconfdir}/authselect/custom/soos
%dir %attr(0755, root, root) %{_sharedstatedir}/soos
%dir %attr(0700, root, root) %{_sharedstatedir}/soos/biometrics
%dir %attr(0700, root, root) %{_sharedstatedir}/soos/evidence
%dir %attr(0755, root, root) %{_sharedstatedir}/soos/models
%ghost %attr(0600, root, root) %{_sharedstatedir}/soos/master.key
%ghost %dir %attr(0750, root, soos) %{_rundir}/soos

%changelog
* Sat Sep 19 2026 soos developers <dev@soos.local> - 0.1.0-1
- Initial packaging release of soos for Fedora and RHEL.
