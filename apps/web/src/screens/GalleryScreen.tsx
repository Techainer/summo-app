import { CircleAlert, Info, OctagonX, TriangleAlert } from "lucide-react";

import {
  Alert,
  Button,
  Card,
  CardBody,
  Checkbox,
  Chip,
  Input,
  Progress,
  SegmentedControl,
  Select,
  Skeleton,
  StatusChip,
  TextArea,
} from "../components/ui";

/**
 * Every primitive, in every state, on one page.
 *
 * `shots.mjs` photographs screens, which is the right level for "does this look broken" and the
 * wrong one for "does a disabled button still meet AA". A screen shows each control in whichever
 * state it happens to be in: nothing in this app is normally rendered busy, or disabled, or holding
 * a Vietnamese sentence long enough to wrap, so those are the states that change without anybody
 * seeing — and they are where the copies this release removed had drifted.
 *
 * Not shipped. The route is added only under `import.meta.env.DEV`, so it costs the release nothing
 * and cannot be reached by a user who guesses the path.
 *
 * The rows are the two things automation can actually judge — `legible.mjs` runs the same overflow
 * and AA contrast checks here as on the real screens — plus a picture for the things it cannot.
 */
const SIZES = ["sm", "md", "lg"] as const;
const VARIANTS = ["primary", "secondary", "ghost", "danger"] as const;

/**
 * A sentence that has to wrap, and one with no spaces to wrap at.
 *
 * `i18n-exempt` on every line of literal text in this file, and the exemption is the point rather
 * than a concession: the page exists to put controls under the kind of content that breaks them.
 * A Vietnamese sentence long enough to wrap and an address with no spaces in it are the two shapes
 * that have actually broken layouts here, and routing them through the catalogue would test the
 * catalogue instead of the primitives — and make the sample text change whenever a translator
 * edited a real string.
 */
const LONG = "Bản tóm tắt đang chờ bạn xác nhận trước khi gửi cho cả phòng"; // i18n-exempt
const UNSPACED = "東京都渋谷区神宮前六丁目三十五番地六号"; // i18n-exempt

/** The rest of the sample text, in one place so each line can carry the exemption. */
const SAMPLE = {
  pick: "một lựa chọn", // i18n-exempt
  count: "với số", // i18n-exempt
  press: "bấm được", // i18n-exempt
  checked: "đã chọn", // i18n-exempt
  unchecked: "chưa chọn", // i18n-exempt
  one: "một", // i18n-exempt
  two: "hai", // i18n-exempt
  action: "hành động", // i18n-exempt
  exampleSmall: "ví dụ nhỏ", // i18n-exempt
  example: "ví dụ", // i18n-exempt
  downloading: "Đang tải", // i18n-exempt
  queued: "Đang xếp hàng", // i18n-exempt
};

/** What each tone means, said twice: once in colour, once in a shape colour-blind readers can see. */
const ALERT_ICON = {
  accent: <Info className="size-4" />,
  danger: <OctagonX className="size-4" />,
  blocked: <TriangleAlert className="size-4" />,
  rec: <CircleAlert className="size-4" />,
};

function Row({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-fg-faint text-micro font-semibold tracking-wider uppercase">{title}</h2>
      <div className="flex flex-wrap items-center gap-2">{children}</div>
    </section>
  );
}

export function GalleryScreen() {
  return (
    <div data-testid="gallery" className="flex flex-col gap-6 p-6">
      {VARIANTS.map((variant) => (
        <Row key={variant} title={`button · ${variant}`}>
          {SIZES.map((size) => (
            <Button key={size} variant={variant} size={size}>
              {size}
            </Button>
          ))}
          <Button variant={variant} disabled>
            disabled
          </Button>
          <Button variant={variant} busy>
            busy
          </Button>
          <Button variant={variant}>{LONG}</Button>
        </Row>
      ))}

      <Row title="field">
        <Input size="sm" placeholder="sm" />
        <Input size="md" placeholder="md" />
        <Input size="md" placeholder="disabled" disabled />
        <Input size="md" defaultValue={UNSPACED} />
        <Select size="md" defaultValue="a">
          <option value="a">{SAMPLE.pick}</option>
          <option value="b">{LONG}</option>
        </Select>
        <Select size="md" disabled>
          <option>disabled</option>
        </Select>
      </Row>

      <Row title="text area">
        <TextArea rows={2} defaultValue={LONG} className="w-72" />
        <TextArea rows={2} disabled defaultValue="disabled" className="w-48" />
      </Row>

      <Row title="chip">
        <Chip>neutral</Chip>
        <Chip on>on</Chip>
        <Chip tone="accent" on>
          accent
        </Chip>
        <Chip tone="ai" on>
          ai
        </Chip>
        <Chip count={12}>{SAMPLE.count}</Chip>
        <Chip onClick={() => {}}>{SAMPLE.press}</Chip>
        <Chip onClick={() => {}} disabled>
          disabled
        </Chip>
      </Row>

      {/* `SAMPLE.checked` rather than the words with a trailing `// i18n-exempt`.
       *
       * Children of a JSX element are *text*, so that marker was never a comment — it rendered, and
       * the checkbox on this page read "đã chọn // i18n-exempt". Three labels here said that. It
       * survived a release because everything below the fields was outside the screenshot: the
       * suite shot `fullPage` on a document that does not scroll, so the only proof of this was in
       * no picture anybody had. The entries in `SAMPLE` for these three already existed, unused. */}
      <Row title="checkbox">
        <Checkbox checked onChange={() => {}}>
          {SAMPLE.checked}
        </Checkbox>
        <Checkbox checked={false} onChange={() => {}}>
          {SAMPLE.unchecked}
        </Checkbox>
        <Checkbox checked disabled onChange={() => {}}>
          disabled
        </Checkbox>
      </Row>

      <Row title="segmented">
        <SegmentedControl
          size="sm"
          label={SAMPLE.exampleSmall} // i18n-exempt
          value="a"
          onChange={() => {}}
          options={[
            { value: "a", label: SAMPLE.one }, // i18n-exempt
            { value: "b", label: SAMPLE.two },
          ]}
        />
        <SegmentedControl
          size="md"
          label={SAMPLE.example} // i18n-exempt
          value="b"
          onChange={() => {}}
          options={[
            { value: "a", label: SAMPLE.one }, // i18n-exempt
            { value: "b", label: LONG.slice(0, 18) },
          ]}
        />
      </Row>

      <Row title="status">
        {(["todo", "running", "done", "blocked", "failed"] as const).map((status) => (
          <StatusChip key={status} status={status} />
        ))}
      </Row>

      <div className="flex flex-col gap-2">
        <h2 className="text-fg-faint text-micro font-semibold tracking-wider uppercase">alert</h2>
        {(["accent", "danger", "blocked", "rec"] as const).map((tone) => (
          <Alert
            key={tone}
            tone={tone}
            // The icon a caller would actually pass. Every tone but `accent` was drawn with a tick,
            // so three of the four alerts on this page announced a failure with the mark for
            // "done" — which is the sort of thing a page of examples is read to decide, and it was
            // answering it wrongly.
            icon={ALERT_ICON[tone]}
            actions={
              <Button size="sm" variant="secondary">
                {SAMPLE.action}
              </Button>
            }
          >
            {tone} — {LONG}
          </Alert>
        ))}
      </div>

      <Row title="progress">
        <div className="w-48">
          <Progress
            install={{
              model: "m",
              name: SAMPLE.downloading,
              state: "downloading",
              done: 4e7,
              total: 1e8,
            }} // i18n-exempt
          />
        </div>
        {/* No total yet, which is what a queued install looks like: the bar has to say "running"
            without being able to say how far. */}
        <div className="w-48">
          <Progress install={{ model: "m", name: SAMPLE.queued, state: "queued" }} />
        </div>
      </Row>

      <Row title="skeleton">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="h-9 w-24" />
      </Row>

      <Card className="max-w-md">
        <CardBody>
          <p className="text-meta text-fg-dim">
            card · {LONG} · {UNSPACED}
          </p>
        </CardBody>
      </Card>
    </div>
  );
}
