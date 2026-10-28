# Hosting AVCWT Server on Unraid with Cloudflare Tunnels

This guide will walk you through hosting the AVCWT synchronization server as a Docker container on your Unraid server and exposing it securely to the internet using a Cloudflare Tunnel.

This setup is highly recommended as it avoids the need to open ports on your home router (port forwarding) and leverages Cloudflare's DDoS protection and SSL/TLS encryption.

## Prerequisites

1.  An **Unraid** server up and running.
2.  The **Community Applications** plugin installed on Unraid.
3.  A **Cloudflare** account.
4.  A **Domain Name** managed by Cloudflare (DNS resolution handled by Cloudflare).

---

## Phase 1: Building and Running the AVCWT Server on Unraid

Since the AVCWT server is relatively simple, the easiest way to deploy it on Unraid is by building the Docker image locally on your Unraid machine and running it.

### Step 1.1: Transfer Files to Unraid

1.  Create a new share on your Unraid server (e.g., `appdata/avcwt-server`).
2.  Copy the following files from the project repository into that folder:
    *   `server.py`
    *   `Dockerfile`
    *   `requirements-server.txt`

### Step 1.2: Build the Docker Image

1.  Open an SSH session or use the web terminal on your Unraid server.
2.  Navigate to the directory where you placed the files:
    ```bash
    cd /mnt/user/appdata/avcwt-server
    ```
3.  Build the Docker image. We'll tag it as `avcwt-server`:
    ```bash
    docker build -t avcwt-server .
    ```

### Step 1.3: Run the Docker Container

We will now run the container, mapping the internal port `8765` to an external port on Unraid.

1.  In the Unraid terminal, run:
    ```bash
    docker run -d \
      --name avcwt-server \
      --restart unless-stopped \
      -p 8765:8765 \
      avcwt-server
    ```
    *Note: If port `8765` is already in use on your Unraid server, you can change the first number (e.g., `-p 9090:8765`), but remember this new port for the Cloudflare setup.*

2.  Verify it's running by checking your Unraid **Docker** tab. You should see `avcwt-server` running.

---

## Phase 2: Setting up Cloudflare Tunnel (cloudflared)

We will use Cloudflare Zero Trust to create a secure tunnel from your Unraid server to Cloudflare's edge network.

### Step 2.1: Create a Tunnel in Cloudflare

1.  Log in to your Cloudflare dashboard and go to **Zero Trust**.
2.  Navigate to **Networks** -> **Tunnels**.
3.  Click **Create a tunnel**.
4.  Choose **Cloudflared** and click Next.
5.  Name your tunnel (e.g., `unraid-tunnel`) and click **Save tunnel**.
6.  You will be presented with a page titled "Install and run a connector". Under the **Docker** environment section, you will see a command like this:
    ```bash
    docker run cloudflare/cloudflared:latest tunnel --no-autoupdate run --token eyJhbGciOiJIUzI1NiIsIn...
    ```
7.  Copy the long token string (everything after `--token `). You will need this for Unraid.

### Step 2.2: Install `cloudflared` on Unraid

1.  Go to the **Apps** tab (Community Applications) on Unraid.
2.  Search for **cloudflared**.
3.  Look for the official or well-supported version (often by `aeleos` or similar). Click **Install**.
4.  In the configuration template:
    *   Set the **Network Type** to `bridge` or your custom docker network.
    *   Find the variable for the **Token** (it might be named `TUNNEL_TOKEN` or just `Token`).
    *   Paste the token you copied from the Cloudflare dashboard.
5.  Click **Apply** to start the container.
6.  Go back to your Cloudflare Zero Trust dashboard. At the bottom of the "Install and run a connector" page, you should see your Unraid server show up as a connected connector. Click **Next**.

---

## Phase 3: Routing Traffic to the AVCWT Server

Now we need to tell Cloudflare to route traffic from a specific domain (e.g., `sync.yourdomain.com`) to the AVCWT server container.

1.  In the Cloudflare Tunnel setup (Step 3: Route traffic), configure the following:
    *   **Public Hostname:**
        *   **Subdomain:** Choose a subdomain (e.g., `sync`, `avcwt`, `watch`).
        *   **Domain:** Select your domain from the dropdown.
    *   **Service:**
        *   **Type:** Select `HTTP` (Cloudflare handles the WebSocket upgrade automatically).
        *   **URL:** Enter the IP address of your Unraid server and the port you mapped the AVCWT server to (e.g., `192.168.1.100:8765`).
2.  Click **Save tunnel**.

### Important Note on WebSockets in Cloudflare
Cloudflare supports WebSockets out of the box on standard plans. However, ensure that WebSockets are not explicitly disabled in your Cloudflare dashboard for your domain:
*   Go to your domain in the standard Cloudflare dashboard.
*   Go to **Network**.
*   Ensure **WebSockets** is toggled ON (it usually is by default).

---

## Phase 4: Connecting the Client

Your AVCWT server is now securely hosted and accessible from the internet!

To connect your AVCWT client to it:
1.  Open the client application.
2.  When prompted for the server address, enter the public hostname you configured (e.g., `sync.yourdomain.com`).
3.  **Crucial:** Make sure your client code connects using `wss://` (WebSocket Secure) instead of `ws://` because Cloudflare enforces HTTPS/SSL.
    *(You may need to update `client.py` to support `wss://` or ensure the user enters the full `wss://sync.yourdomain.com` URL if your client supports it).*
