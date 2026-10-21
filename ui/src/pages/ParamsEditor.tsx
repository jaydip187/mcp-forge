import { Braces, PlusIcon, Trash2Icon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SCHEMA_TYPES, schemaType, typedToText, type SchemaType } from "@/lib/template";
import { LOCATIONS, type JsonObject, type ParamDef, type ParamLocation } from "@/types";

/** Editable row: the param plus the raw text of its value boxes. */
export interface ParamRow {
  key: string;
  def: ParamDef;
  fixedOn: boolean;
  fixedText: string;
  schemaOpen: boolean;
  schemaText: string;
  error?: string;
}

let nextKey = 0;
export function toRow(def: ParamDef): ParamRow {
  return {
    key: `r${nextKey++}`,
    def,
    fixedOn: def.fixed_value !== null && def.fixed_value !== undefined,
    fixedText: typedToText(def.fixed_value),
    schemaOpen: schemaType(def.schema) === "custom",
    schemaText: JSON.stringify(def.schema, null, 2),
  };
}

export function emptyRow(location: ParamLocation = "query"): ParamRow {
  return toRow({ name: "", location, schema: { type: "string" }, required: location === "path", description: null, fixed_value: null });
}

const LOCATION_LABEL: Record<ParamLocation, string> = { path: "Path", query: "Query", header: "Header", body: "Body" };

export function ParamsEditor({
  rows,
  onChange,
  serverErrors,
}: {
  rows: ParamRow[];
  onChange: (rows: ParamRow[]) => void;
  serverErrors: (index: number) => string[];
}) {
  const update = (i: number, patch: Partial<ParamRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch, error: undefined } : r)));
  const updateDef = (i: number, patch: Partial<ParamDef>) => {
    const row = rows[i];
    if (row) update(i, { def: { ...row.def, ...patch } });
  };

  function setType(i: number, t: SchemaType) {
    const row = rows[i];
    if (!row) return;
    const schema: JsonObject = { ...row.def.schema, type: t };
    if (t !== "array") delete schema.items;
    update(i, { def: { ...row.def, schema }, schemaText: JSON.stringify(schema, null, 2) });
  }

  return (
    <div className="space-y-3">
      {rows.length === 0 && (
        <p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
          No parameters. Add one for each value the agent should fill in, or that you want to fix.
        </p>
      )}
      <ul className="divide-y rounded-lg border bg-card">
        {rows.map((row, i) => {
          const id = `param-${row.key}`;
          const type = schemaType(row.def.schema);
          const errors = [...(row.error ? [row.error] : []), ...serverErrors(i)];
          return (
            <li key={row.key} className="space-y-3 p-3" aria-label={`Parameter ${row.def.name || i + 1}`}>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-[minmax(8rem,1fr)_7rem_7.5rem_auto_auto]">
                <div className="col-span-2 grid gap-1 sm:col-span-1">
                  <Label htmlFor={`${id}-name`} className="text-xs text-muted-foreground">
                    Name
                  </Label>
                  <Input id={`${id}-name`} value={row.def.name} onChange={(e) => updateDef(i, { name: e.target.value })} className="font-mono" placeholder="user_id" aria-invalid={errors.length > 0} />
                </div>
                <div className="grid gap-1">
                  <Label htmlFor={`${id}-loc`} className="text-xs text-muted-foreground">
                    Goes in
                  </Label>
                  <Select value={row.def.location} onValueChange={(v) => updateDef(i, { location: v as ParamLocation, ...(v === "path" ? { required: true } : {}) })}>
                    <SelectTrigger id={`${id}-loc`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {LOCATIONS.map((l) => (
                        <SelectItem key={l} value={l}>
                          {LOCATION_LABEL[l]}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor={`${id}-type`} className="text-xs text-muted-foreground">
                    Type
                  </Label>
                  <Select value={type} onValueChange={(v) => v !== "custom" && setType(i, v as SchemaType)}>
                    <SelectTrigger id={`${id}-type`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SCHEMA_TYPES.map((t) => (
                        <SelectItem key={t} value={t}>
                          {t}
                        </SelectItem>
                      ))}
                      {type === "custom" && <SelectItem value="custom">custom</SelectItem>}
                    </SelectContent>
                  </Select>
                </div>
                <div className="flex items-end gap-2 pb-2">
                  <Checkbox id={`${id}-req`} checked={row.def.required} onCheckedChange={(v) => updateDef(i, { required: v === true })} />
                  <Label htmlFor={`${id}-req`} className="text-sm">
                    Required
                  </Label>
                </div>
                <div className="flex items-end justify-end gap-1">
                  <Button type="button" size="icon" variant={row.schemaOpen ? "secondary" : "ghost"} onClick={() => update(i, { schemaOpen: !row.schemaOpen })} aria-label="Edit JSON schema" aria-pressed={row.schemaOpen}>
                    <Braces />
                  </Button>
                  <Button type="button" size="icon" variant="ghost" onClick={() => onChange(rows.filter((_, j) => j !== i))} aria-label="Remove parameter">
                    <Trash2Icon />
                  </Button>
                </div>
              </div>

              <div className="grid gap-2 sm:grid-cols-[1fr_minmax(12rem,16rem)]">
                <Input value={row.def.description ?? ""} onChange={(e) => updateDef(i, { description: e.target.value || null })} placeholder="What the agent should put here" aria-label="Description for the agent" />
                <div className="flex items-center gap-2">
                  <Switch id={`${id}-fixed`} checked={row.fixedOn} onCheckedChange={(v) => update(i, { fixedOn: v })} />
                  <Label htmlFor={`${id}-fixed`} className="shrink-0 text-sm">
                    Fixed value
                  </Label>
                  {row.fixedOn && (
                    <Input value={row.fixedText} onChange={(e) => update(i, { fixedText: e.target.value })} className="font-mono" aria-label="Fixed value" placeholder={type === "string" ? "value" : "JSON"} />
                  )}
                </div>
              </div>
              {row.fixedOn && (
                <Badge variant="secondary" className="font-normal">
                  Always sent as set. Hidden from the agent.
                </Badge>
              )}

              {row.schemaOpen && (
                <div className="grid gap-1">
                  <Label htmlFor={`${id}-schema`} className="text-xs text-muted-foreground">
                    JSON Schema for this value
                  </Label>
                  <Textarea id={`${id}-schema`} value={row.schemaText} onChange={(e) => update(i, { schemaText: e.target.value })} className="min-h-24 font-mono text-xs" spellCheck={false} />
                </div>
              )}

              {errors.map((m, k) => (
                <p key={k} className="text-xs text-destructive">
                  {m}
                </p>
              ))}
            </li>
          );
        })}
      </ul>
      <Button type="button" variant="outline" size="sm" onClick={() => onChange([...rows, emptyRow()])}>
        <PlusIcon /> Add parameter
      </Button>
    </div>
  );
}
