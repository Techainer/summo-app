import { Check, Info } from "lucide-react";

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

/** A sentence that has to wrap, and one with no spaces to wrap at. */
const LONG = "Bản tóm tắt đang chờ bạn xác nhận trước khi gửi cho cả phòng";
const UNSPACED = "東京都渋谷区神宮前六丁目三十五番地六号";

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
          <option value="a">một lựa chọn</option>
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
        <Chip count={12}>với số</Chip>
        <Chip onClick={() => {}}>bấm được</Chip>
        <Chip onClick={() => {}} disabled>
          disabled
        </Chip>
      </Row>

      <Row title="checkbox">
        <Checkbox checked onChange={() => {}}>
          đã chọn
        </Checkbox>
        <Checkbox checked={false} onChange={() => {}}>
          chưa chọn
        </Checkbox>
        <Checkbox checked disabled onChange={() => {}}>
          disabled
        </Checkbox>
      </Row>

      <Row title="segmented">
        <SegmentedControl
          size="sm"
          label="ví dụ nhỏ"
          value="a"
          onChange={() => {}}
          options={[
            { value: "a", label: "một" },
            { value: "b", label: "hai" },
          ]}
        />
        <SegmentedControl
          size="md"
          label="ví dụ"
          value="b"
          onChange={() => {}}
          options={[
            { value: "a", label: "một" },
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
            icon={tone === "accent" ? <Info className="size-4" /> : <Check className="size-4" />}
            actions={
              <Button size="sm" variant="secondary">
                hành động
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
            install={{ model: "m", name: "Đang tải", state: "downloading", done: 4e7, total: 1e8 }}
          />
        </div>
        {/* No total yet, which is what a queued install looks like: the bar has to say "running"
            without being able to say how far. */}
        <div className="w-48">
          <Progress install={{ model: "m", name: "Đang xếp hàng", state: "queued" }} />
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
