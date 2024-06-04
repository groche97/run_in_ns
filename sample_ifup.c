#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <string.h>
#include <time.h>
#include <errno.h>

#include <netinet/in.h>
#include <netinet/tcp.h>

#include <linux/sockios.h>
#include <linux/if.h>
#include <linux/if_link.h>
#include <linux/rtnetlink.h>

#define ALIGNTO		4
#define ALIGN(len)		(((len)+ALIGNTO-1) & ~(ALIGNTO-1))
#define ATTR_HDRLEN	ALIGN(sizeof(struct nlattr))
#define SOCKET_BUFFER_SIZE (sysconf(_SC_PAGESIZE) < 8192L ? sysconf(_SC_PAGESIZE) : 8192L)

int main()
{
	int nls = -1;
	struct sockaddr_nl kernel_nladdr;
	struct iovec io;
	struct msghdr msg;
	struct ifinfomsg *ifm;
	unsigned int change, flags, seq;
	char *ifname;
	char buf[SOCKET_BUFFER_SIZE]; /* 8192 by default */

	struct nlmsghdr *nlmsg;
	seq = time(NULL);

	/* The netlink message is destined to the kernel so nl_pid == 0. */
	memset(&kernel_nladdr, 0, sizeof(kernel_nladdr));
	kernel_nladdr.nl_family = AF_NETLINK;
	kernel_nladdr.nl_groups = 0; /* unicast */
	kernel_nladdr.nl_pid = 0;

	nls = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
	if (nls == -1)
	{
		printf("cannot open socket %s\n", strerror(errno));
		return -1;
	}

	int br;

	br = bind(nls, (struct sockaddr *) &kernel_nladdr, sizeof (kernel_nladdr));
	if (br == -1)
	{
		printf("cannot bind to socket\n");
		return -1;
	}

	int hlen = ALIGN(sizeof(struct nlmsghdr));
	nlmsg = (struct nlmsghdr *) buf;
	memset(buf, 0, hlen);
	nlmsg->nlmsg_len = hlen;

	nlmsg->nlmsg_type = RTM_NEWLINK;
	nlmsg->nlmsg_flags = NLM_F_REQUEST | NLM_F_ACK;
	nlmsg->nlmsg_seq = seq;

	/* extra header */
	char *ptr = (char *)nlmsg + nlmsg->nlmsg_len;
	size_t ehlen = ALIGN(sizeof(*ifm));
	nlmsg->nlmsg_len += ehlen;
	memset(ptr, 0, ehlen);

	/* put interface down */
	change = 0;
	flags = 0;
	change |= IFF_UP;
	flags &= ~IFF_UP; /* down = !up, obviously */

	ifm = (void *)ptr;
	ifm->ifi_family = AF_UNSPEC;
	ifm->ifi_change = change;
	ifm->ifi_flags = flags;

	/* add payload details - nlattr & padding */
	ifname = "test";
	struct nlattr *attr = (void *)nlmsg + ALIGN(nlmsg->nlmsg_len);
	uint16_t payload_len = ALIGN(sizeof(struct nlattr)) + strlen(ifname);
	int pad;

	attr->nla_type = IFLA_IFNAME;
	attr->nla_len = payload_len;
	memcpy((void *)attr + ATTR_HDRLEN, ifname, strlen(ifname));
	pad = ALIGN(strlen(ifname)) - strlen(ifname);
	if (pad > 0)
		memset((void *)attr + ATTR_HDRLEN + strlen(ifname), 0, pad);

	nlmsg->nlmsg_len += ALIGN(payload_len);

	/* end of inner netlink nlattr details */

	/* Stick the request in an io vector */
	io.iov_base = (void *)nlmsg;
	io.iov_len = nlmsg->nlmsg_len;

	/* Wrap it in a msg */
	memset(&msg, 0, sizeof(msg));
	msg.msg_iov = &io;
	msg.msg_iovlen = 1;
	msg.msg_name = (void *)&kernel_nladdr;
	msg.msg_namelen = sizeof(kernel_nladdr);

	/* Send it */
	int res = sendmsg(nls, &msg, 0);
	printf("result of send: %d", res);

	return 0;
}
