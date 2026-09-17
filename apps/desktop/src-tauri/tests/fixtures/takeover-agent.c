/* Test-only local agent. Direct mode can deliberately replace a socket to
 * simulate a competing launcher; desktop-launch mode refuses live sockets. */
#include <arpa/inet.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

static int read_all(int fd, void *buffer, unsigned size) {
    unsigned offset = 0;
    while (offset < size) {
        int count = read(fd, (char *)buffer + offset, size - offset);
        if (count <= 0) return 0;
        offset += count;
    }
    return 1;
}

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    signal(SIGPIPE, SIG_IGN);
    int launched = strcmp(argv[1], "--state-dir") == 0;
    const char *path = launched ? argv[4] : argv[1];
    int version = launched ? PROTOCOL_VERSION : atoi(argv[3]);
    struct sockaddr_un address = {0};
    address.sun_family = AF_UNIX;
    if (strlen(path) >= sizeof address.sun_path) return 3;
    strcpy(address.sun_path, path);
    if (launched) {
        int probe = socket(AF_UNIX, SOCK_STREAM, 0);
        if (connect(probe, (struct sockaddr *)&address, sizeof address) == 0) return 4;
        close(probe);
    }
    unlink(path);
    int server = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(server, (struct sockaddr *)&address, sizeof address) != 0 ||
        chmod(path, 0600) != 0 || listen(server, 32) != 0) return 5;
    if (!launched) {
        int marker = open(argv[2], O_CREAT | O_WRONLY, 0600);
        if (marker < 0) return 6;
        close(marker);
    }
    for (;;) {
        int client = accept(server, NULL, NULL);
        if (client < 0) continue;
        uint32_t length;
        char request[4096];
        if (!read_all(client, &length, 4) || ntohl(length) >= sizeof request ||
            !read_all(client, request, ntohl(length))) { close(client); continue; }
        request[ntohl(length)] = '\0';
        char *id = strstr(request, "\"id\":");
        char response[256];
        int count = snprintf(response, sizeof response,
            "{\"version\":%d,\"id\":%llu,\"status\":\"success\",\"value\":{\"state\":\"ready\"}}",
            version, id ? strtoull(id + 5, NULL, 10) : 0);
        length = htonl(count);
        write(client, &length, 4);
        write(client, response, count);
        close(client);
    }
}
