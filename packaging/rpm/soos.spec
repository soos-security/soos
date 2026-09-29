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
install -m 0755 scripts/provision_master_key.sh %{buildroot}/usr/libexec/soos/provision-master-key

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
# Configuration directory (disable flags, recorded authselect profile)
install -d -m 0755 %{buildroot}%{_sysconfdir}/soos

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

# Generate the cryptographic master key on this host if absent (32 bytes,
# mode 0600 root:root). Never packaged (%ghost, GitHub #144): the shipped helper
# creates it with mode 0600 from inception and never overwrites an existing key.
/usr/libexec/soos/provision-master-key --state-dir "%{_sharedstatedir}/soos"

# Ensure runtime socket directory exists with proper permissions
mkdir -p %{_rundir}/soos
chmod 0750 %{_rundir}/soos || :
chown root:soos %{_rundir}/soos 2>/dev/null || :

# Record the selected authselect profile (id + features) so that %preun can
# restore it once custom/soos has been activated. The profile is never
# activated automatically: run 'authselect select custom/soos with-faillock --force'.
if command -v authselect >/dev/null 2>&1; then
    current="$(authselect current --raw 2>/dev/null || :)"
    case "${current}" in
        ""|custom/soos*) : ;;
        *)
            mkdir -p %{_sysconfdir}/soos || :
            echo "${current}" > %{_sysconfdir}/soos/authselect.previous || :
            chmod 0644 %{_sysconfdir}/soos/authselect.previous || :
            ;;
    esac
fi

%preun
%systemd_preun soos-daemon.service

# On package removal, leave authselect on a valid profile before the custom
# profile files disappear: restore the recorded profile, else a stock one.
if [ "$1" -eq 0 ] && command -v authselect >/dev/null 2>&1; then
    current="$(authselect current --raw 2>/dev/null || :)"
    case "${current}" in
        custom/soos*)
            previous=""
            if [ -f %{_sysconfdir}/soos/authselect.previous ]; then
                previous="$(head -n 1 %{_sysconfdir}/soos/authselect.previous | tr -cd 'A-Za-z0-9/_. -')"
            fi
            case "${previous}" in
                ""|custom/soos*) previous="local" ;;
            esac
            # Word splitting intended: profile id followed by feature names.
            authselect select ${previous} --force >/dev/null 2>&1 \
                || authselect select local --force >/dev/null 2>&1 \
                || authselect select minimal --force >/dev/null 2>&1 \
                || authselect select sssd --force >/dev/null 2>&1 \
                || echo "soos: unable to restore an authselect profile, run 'authselect select <profile> --force' manually" >&2
            ;;
    esac
    rm -f %{_sysconfdir}/soos/authselect.previous || :
fi

%postun
%systemd_postun_with_restart soos-daemon.service

%files
/usr/libexec/soos/soos-daemon
/usr/libexec/soos/provision-master-key
%{_bindir}/soos-admin
%{_bindir}/soos-enroll
%{_libdir}/security/pam_soos.so
%{_unitdir}/soos-daemon.service
%{_sysconfdir}/authselect/custom/soos
%dir %attr(0755, root, root) %{_sysconfdir}/soos
%ghost %{_sysconfdir}/soos/authselect.previous
%dir %attr(0755, root, root) %{_sharedstatedir}/soos
%dir %attr(0700, root, root) %{_sharedstatedir}/soos/biometrics
%dir %attr(0700, root, root) %{_sharedstatedir}/soos/evidence
%dir %attr(0755, root, root) %{_sharedstatedir}/soos/models
%ghost %attr(0600, root, root) %{_sharedstatedir}/soos/master.key
%ghost %dir %attr(0750, root, soos) %{_rundir}/soos

%changelog
* Sat Sep 19 2026 soos developers <dev@soos.local> - 0.1.0-1
- Initial packaging release of soos for Fedora and RHEL.
