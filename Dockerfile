FROM archlinux as root
RUN pacman -Syu --noconfirm git sudo base-devel \
    && useradd -m -u 1000 updater \
    && echo 'updater ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/updater

FROM root as yay
USER updater
RUN cd /tmp \
    && curl -Lo yay.tar.gz https://aur.archlinux.org/cgit/aur.git/snapshot/yay.tar.gz \
    && tar zxvf yay.tar.gz \
    && cd yay \
    && makepkg -si --rmdeps --noconfirm

FROM root
COPY --from=yay /usr/bin/yay /usr/bin/yay
USER updater
RUN git config --global user.email "you@example.com" \
    && git config --global user.name "Your Name" \
    && mkdir ~/.ssh \
    && echo -e "Host aur.archlinux.org\n  IdentityFile ~/.ssh/aur\n  User aur" > ~/.ssh/config
