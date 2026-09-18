
const http = require("http");
const fs = require("fs");
const path = require("path");
const ROOT = "C:/_Project/AudioLink/docs/design/preview";
http.createServer((req, res) => {
  const rel = decodeURIComponent((req.url || "/").split("?")[0]);
  const file = path.join(ROOT, rel === "/" ? "index.html" : rel);
  fs.readFile(file, (err, data) => {
    if (err) {
      // 目录列表，方便挑文件
      fs.readdir(ROOT, (e2, files) => {
        if (e2) { res.writeHead(404); res.end("404"); return; }
        res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
        res.end("<meta charset=utf-8><ul>" + files.map((f) => '<li><a href="/' + f + '">' + f + "</a></li>").join("") + "</ul>");
      });
      return;
    }
    const type = file.endsWith(".html") ? "text/html; charset=utf-8" : "text/plain; charset=utf-8";
    res.writeHead(200, { "Content-Type": type });
    res.end(data);
  });
}).listen(8799, "127.0.0.1", () => console.log("preview server on http://127.0.0.1:8799/"));
