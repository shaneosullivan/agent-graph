import "./site.css";

import Link from "next/link";

import { Brand } from "./brand";

export default function NotFound() {
  return (
    <div className="site">
      <div className="page">
        <header className="top">
          <Brand />
        </header>
        <div className="card narrow">
          <div className="card-body">
            <h1>There&rsquo;s nothing here</h1>
            <p>This link doesn&rsquo;t match a shared log. Check you have the whole address.</p>
            <Link
              className="button"
              href="/"
              style={{ display: "block", textAlign: "center", textDecoration: "none" }}
            >
              Share a log
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
}
