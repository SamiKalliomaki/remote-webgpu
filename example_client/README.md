# example_client

A web page with a full-screen canvas that connects to the remote WebGPU
server using `remote-webgpu-client`.

```sh
# once: install (also installs ../client via file: dependency)
npm install

# bundle + serve on http://127.0.0.1:8000 with rebuild-on-reload
npm run serve

# or just produce dist/main.js and serve index.html any other way
npm run build
```

Start the native server first (`../example_server`, listening on port 8080
by default), then open the page.  Use `?server=ws://host:port` to point it
at a different server.  Connection progress is shown in the overlay at the
bottom-left; the canvas shows whatever the server renders (the spinning red
triangle).  Note the canvas backing size is dictated by the server's window
size once connected.
