import "../site.css";

import type {Metadata} from "next";

import {CONTACT} from "@/lib/contact";

import {InfoPage} from "../info-page";

export const metadata: Metadata = {
  title: "Contact · Agent Graph",
  description: "How to get in touch about Agent Graph.",
};

export default function Contact() {
  return (
    <InfoPage title="Contact">
      <p>
        Questions, problems, ideas, or something about your account or a
        subscription: we&rsquo;d like to hear from you.
      </p>
      <dl className="contact-list">
        <dt>Email</dt>
        <dd>
          <a href={`mailto:${CONTACT.email}`}>{CONTACT.email}</a>
        </dd>
        <dt>X (Twitter)</dt>
        <dd>
          <a href={CONTACT.twitter.url}>{CONTACT.twitter.handle}</a>
        </dd>
        <dt>Threads</dt>
        <dd>
          <a href={CONTACT.threads.url}>{CONTACT.threads.handle}</a>
        </dd>
        <dt>Bugs and features</dt>
        <dd>
          <a href={`${CONTACT.source}/issues`}>GitHub issues</a>
        </dd>
      </dl>
      <p className="muted">
        Please don&rsquo;t send the contents of a log, or a live share&rsquo;s
        link, by email: tell us about it instead, and we&rsquo;ll ask for what
        we need.
      </p>
    </InfoPage>
  );
}
