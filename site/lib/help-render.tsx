import {Fragment} from "react";

import {link} from "./links";

/**
 * Renders the CLI's help text (lib/cli-help.json, the same file compiled into
 * the `agent-graph` binary) as HTML. The binary lays the same blocks out for a
 * terminal in build.rs.
 */

export type Block =
  | {text: string}
  | {heading: string}
  | {list: Array<string>}
  | {steps: Array<{command?: string; text: string}>}
  | {table: Array<[string, string]>}
  | {code: string};

export type Example = {command: string; text: string};
export type Option = {id: string; flag: string; text: string};

export type Command = {
  name: string;
  summary: string;
  description: Array<Block>;
  options: Array<Option>;
  examples: Array<Example>;
};

export type Help = {
  program: string;
  summary: string;
  description: Array<Block>;
  quickStart: Array<string>;
  sections: Array<{
    title: string;
    blocks?: Array<Block>;
    examples?: Array<Example>;
  }>;
  footer: string;
  commands: Array<Command>;
};

/** Text with `code spans` and bare https:// links. */
export function Inline({text}: {text: string}) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 === 1 ? (
          <code key={i}>{part}</code>
        ) : (
          <Fragment key={i}>
            {part.split(/(https:\/\/[^\s,)]+)/).map((piece, j) =>
              piece.startsWith("https://") ? (
                <a key={j} {...link(piece.replace(/\.$/, ""))}>
                  {piece.replace(/\.$/, "")}
                </a>
              ) : (
                piece
              ),
            )}
          </Fragment>
        ),
      )}
    </>
  );
}

/** Table terms that look like code (paths, flags, variables) are shown as code. */
function Term({term}: {term: string}) {
  return /[/_=<>[\]~.-]/.test(term) ? (
    <code>{term}</code>
  ) : (
    <strong>{term}</strong>
  );
}

export function Blocks({blocks}: {blocks: Array<Block>}) {
  return (
    <>
      {blocks.map((block, i) => {
        if ("text" in block) {
          return (
            <p key={i}>
              <Inline text={block.text} />
            </p>
          );
        }
        if ("heading" in block) {
          return <h4 key={i}>{block.heading}</h4>;
        }
        if ("list" in block) {
          return (
            <ul key={i}>
              {block.list.map((item, j) => (
                <li key={j}>
                  <Inline text={item} />
                </li>
              ))}
            </ul>
          );
        }
        if ("steps" in block) {
          return (
            <ol key={i} className="steps">
              {block.steps.map((step, j) => (
                <li key={j}>
                  {step.command && <code className="cmd">{step.command}</code>}
                  <span>
                    <Inline text={step.text} />
                  </span>
                </li>
              ))}
            </ol>
          );
        }
        if ("table" in block) {
          return (
            <table key={i} className="terms">
              <tbody>
                {block.table.map(([term, text], j) => (
                  <tr key={j}>
                    <th scope="row">
                      <Term term={term} />
                    </th>
                    <td>
                      <Inline text={text} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          );
        }
        return (
          <pre key={i} className="block-code">
            {block.code}
          </pre>
        );
      })}
    </>
  );
}

export function Examples({examples}: {examples: Array<Example>}) {
  return (
    <div className="examples">
      {examples.map((example, i) => (
        <div key={i} className="example">
          <pre>
            <span className="prompt" aria-hidden="true">
              ${" "}
            </span>
            {example.command}
          </pre>
          <p>
            <Inline text={example.text} />
          </p>
        </div>
      ))}
    </div>
  );
}

export function Options({options}: {options: Array<Option>}) {
  return (
    <table className="terms options">
      <tbody>
        {options.map(option => (
          <tr key={option.id}>
            <th scope="row">
              <code>{option.flag}</code>
            </th>
            <td>
              <Inline text={option.text} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
