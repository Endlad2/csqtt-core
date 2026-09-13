// SPDX-License-Identifier: MIT
// Creates one Linux TUN device and passes its FD to CSQTT over abstract UDS.
// It never creates addresses, routes, firewall rules, or DNS settings.

#define _GNU_SOURCE
#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/if_tun.h>
#include <net/if.h>
#include <signal.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

enum {
    DEFAULT_TIMEOUT_SECONDS = 30,
    MAX_TIMEOUT_SECONDS = 300,
    RETRIES_PER_SECOND = 10,
    RETRY_DELAY_NANOSECONDS = 100000000L,
};

static volatile sig_atomic_t keep_running = 1;

static void stop(int signal_number) {
    (void)signal_number;
    keep_running = 0;
}

static void usage(const char *program) {
    (void)fprintf(stderr, "Usage: %s --interface NAME --uds NAME [--timeout SECONDS] [--up]\n", program);
}

static int close_fd(const int fd, const char *description) {
    if (fd < 0) {
        return 0;
    }
    if (close(fd) == -1) {
        perror(description);
        return -1;
    }
    return 0;
}

static int parse_timeout(const char *value, unsigned int *result) {
    char *end = NULL;
    unsigned long parsed = 0U;

    if (value == NULL || result == NULL || value[0] == '\0') {
        return -1;
    }
    errno = 0;
    parsed = strtoul(value, &end, 10);
    if (errno == ERANGE || end == value || *end != '\0' || parsed == 0U ||
        parsed > MAX_TIMEOUT_SECONDS || parsed > UINT_MAX) {
        return -1;
    }
    *result = (unsigned int)parsed;
    return 0;
}

static int install_signal_handlers(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = stop;
    if (sigemptyset(&action.sa_mask) == -1) {
        perror("sigemptyset");
        return -1;
    }
    if (sigaction(SIGINT, &action, NULL) == -1 || sigaction(SIGTERM, &action, NULL) == -1) {
        perror("sigaction");
        return -1;
    }
    return 0;
}

static int copy_interface_name(struct ifreq *request, const char *name) {
    const size_t length = strnlen(name, IFNAMSIZ);
    if (length == 0U || length == IFNAMSIZ) {
        (void)fprintf(stderr, "invalid interface name\n");
        return -1;
    }
    memset(request, 0, sizeof(*request));
    memcpy(request->ifr_name, name, length);
    request->ifr_flags = IFF_TUN | IFF_NO_PI;
    return 0;
}

static int set_interface_up(struct ifreq *request) {
    int control_fd = -1;
    int result = -1;

    control_fd = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (control_fd == -1) {
        perror("control socket");
        goto cleanup;
    }
    if (ioctl(control_fd, SIOCGIFFLAGS, request) == -1) {
        perror("SIOCGIFFLAGS");
        goto cleanup;
    }
    request->ifr_flags |= IFF_UP;
    if (ioctl(control_fd, SIOCSIFFLAGS, request) == -1) {
        perror("SIOCSIFFLAGS");
        goto cleanup;
    }
    result = 0;

cleanup:
    if (close_fd(control_fd, "close control socket") == -1) {
        result = -1;
    }
    return result;
}

static int open_tun(const char *name, const bool set_up) {
    int tun_fd = -1;
    struct ifreq request;
    size_t name_length = 0U;

    if (name == NULL || copy_interface_name(&request, name) == -1) {
        return -1;
    }
    name_length = strnlen(name, IFNAMSIZ);
    tun_fd = open("/dev/net/tun", O_RDWR | O_CLOEXEC);
    if (tun_fd == -1) {
        perror("open /dev/net/tun");
        return -1;
    }
    if (ioctl(tun_fd, TUNSETIFF, &request) == -1) {
        perror("TUNSETIFF");
        (void)close_fd(tun_fd, "close TUN");
        return -1;
    }
    if (memcmp(request.ifr_name, name, name_length) != 0 || request.ifr_name[name_length] != '\0') {
        (void)fprintf(stderr, "kernel returned unexpected interface name\n");
        (void)close_fd(tun_fd, "close TUN");
        return -1;
    }
    if (set_up && set_interface_up(&request) == -1) {
        (void)close_fd(tun_fd, "close TUN");
        return -1;
    }
    return tun_fd;
}

static int make_abstract_address(const char *name, struct sockaddr_un *address, socklen_t *address_length) {
    const size_t capacity = sizeof(address->sun_path) - 1U;
    size_t name_length = 0U;

    if (name == NULL || address == NULL || address_length == NULL) {
        return -1;
    }
    name_length = strnlen(name, capacity + 1U);
    if (name_length == 0U || name_length > capacity) {
        (void)fprintf(stderr, "invalid UDS name\n");
        return -1;
    }
    memset(address, 0, sizeof(*address));
    address->sun_family = AF_UNIX;
    memcpy(address->sun_path + 1, name, name_length);
    *address_length = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + 1U + name_length);
    return 0;
}

static int sleep_before_retry(void) {
    const struct timespec delay = {.tv_sec = 0, .tv_nsec = RETRY_DELAY_NANOSECONDS};
    struct timespec remaining = delay;

    while (nanosleep(&remaining, &remaining) == -1) {
        if (errno != EINTR) {
            perror("nanosleep");
            return -1;
        }
        if (keep_running == 0) {
            return -1;
        }
    }
    return 0;
}

static int verify_root_peer(const int socket_fd) {
    struct ucred peer;
    socklen_t peer_length = sizeof(peer);

    if (getsockopt(socket_fd, SOL_SOCKET, SO_PEERCRED, &peer, &peer_length) == -1) {
        perror("SO_PEERCRED");
        return -1;
    }
    if (peer_length != sizeof(peer) || peer.uid != 0U || peer.pid <= 0) {
        (void)fprintf(stderr, "CSQTT UDS peer is not root\n");
        errno = EPERM;
        return -1;
    }
    return 0;
}

static int connect_abstract(const char *name, const unsigned int timeout) {
    struct sockaddr_un address;
    socklen_t address_length = 0;
    const unsigned int attempts = timeout * RETRIES_PER_SECOND;

    if (make_abstract_address(name, &address, &address_length) == -1) {
        return -1;
    }
    for (unsigned int attempt = 0U; attempt < attempts && keep_running != 0; ++attempt) {
        const int socket_fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (socket_fd == -1) {
            perror("UDS socket");
            return -1;
        }
        if (connect(socket_fd, (const struct sockaddr *)&address, address_length) == 0) {
            if (verify_root_peer(socket_fd) == 0) {
                return socket_fd;
            }
            (void)close_fd(socket_fd, "close UDS socket");
            return -1;
        }
        const int connect_error = errno;
        (void)close_fd(socket_fd, "close UDS socket");
        if (connect_error != ECONNREFUSED && connect_error != ENOENT && connect_error != EINTR) {
            errno = connect_error;
            perror("connect CSQTT UDS");
            return -1;
        }
        if (sleep_before_retry() == -1) {
            return -1;
        }
    }
    (void)fprintf(stderr, "timeout waiting for CSQTT UDS\n");
    return -1;
}

static int send_fd(const int socket_fd, const int tun_fd) {
    char payload = 'T';
    struct iovec iov = {.iov_base = &payload, .iov_len = sizeof(payload)};
    char control[CMSG_SPACE(sizeof(tun_fd))];
    struct msghdr message;
    struct cmsghdr *header = NULL;

    memset(control, 0, sizeof(control));
    memset(&message, 0, sizeof(message));
    message.msg_iov = &iov;
    message.msg_iovlen = 1U;
    message.msg_control = control;
    message.msg_controllen = sizeof(control);
    header = CMSG_FIRSTHDR(&message);
    if (header == NULL) {
        (void)fprintf(stderr, "cannot construct SCM_RIGHTS message\n");
        return -1;
    }
    header->cmsg_level = SOL_SOCKET;
    header->cmsg_type = SCM_RIGHTS;
    header->cmsg_len = CMSG_LEN(sizeof(tun_fd));
    memcpy(CMSG_DATA(header), &tun_fd, sizeof(tun_fd));

    for (;;) {
        const ssize_t sent = sendmsg(socket_fd, &message, MSG_NOSIGNAL);
        if (sent == (ssize_t)sizeof(payload)) {
            return 0;
        }
        if (sent == -1 && errno == EINTR) {
            continue;
        }
        if (sent >= 0) {
            errno = EIO;
        }
        perror("send TUN FD");
        return -1;
    }
}

int main(int argc, char **argv) {
    const char *interface = NULL;
    const char *uds = NULL;
    unsigned int timeout = DEFAULT_TIMEOUT_SECONDS;
    bool set_up = false;
    int tun_fd = -1;
    int uds_fd = -1;
    int result = EXIT_FAILURE;

    for (int index = 1; index < argc; ++index) {
        if (strcmp(argv[index], "--interface") == 0 && index < argc - 1) {
            interface = argv[++index];
        } else if (strcmp(argv[index], "--uds") == 0 && index < argc - 1) {
            uds = argv[++index];
        } else if (strcmp(argv[index], "--timeout") == 0 && index < argc - 1) {
            if (parse_timeout(argv[++index], &timeout) == -1) {
                usage(argv[0]);
                return EXIT_FAILURE;
            }
        } else if (strcmp(argv[index], "--up") == 0) {
            set_up = true;
        } else {
            usage(argv[0]);
            return EXIT_FAILURE;
        }
    }
    if (interface == NULL || uds == NULL || install_signal_handlers() == -1) {
        usage(argv[0]);
        return EXIT_FAILURE;
    }
    tun_fd = open_tun(interface, set_up);
    if (tun_fd == -1) {
        goto cleanup;
    }
    uds_fd = connect_abstract(uds, timeout);
    if (uds_fd == -1) {
        goto cleanup;
    }
    if (send_fd(uds_fd, tun_fd) == -1) {
        goto cleanup;
    }
    if (close_fd(uds_fd, "close UDS socket") == -1) {
        uds_fd = -1;
        goto cleanup;
    }
    uds_fd = -1;
    (void)fprintf(stderr, "attached interface %s to CSQTT\n", interface);
    while (keep_running != 0) {
        if (pause() == -1 && errno != EINTR) {
            perror("pause");
            goto cleanup;
        }
    }
    result = EXIT_SUCCESS;

cleanup:
    if (close_fd(uds_fd, "close UDS socket") == -1) {
        result = EXIT_FAILURE;
    }
    if (close_fd(tun_fd, "close TUN") == -1) {
        result = EXIT_FAILURE;
    }
    return result;
}
