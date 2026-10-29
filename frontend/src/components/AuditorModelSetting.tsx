import { AUDITOR_MODELS, AuditorModel, MODEL_HINT, auditorModelLabel, isAuditorModel } from '../auditor';

/** Settings → the model audit turns run on (`auditor_model`, §18). A 400 about it shows right here. */
export function AuditorModelSetting({ value, error, onChange }: { value: string; error: string | null; onChange: (m: AuditorModel) => void }) {
  return (
    <div className="mt-2" data-testid="auditor-model">
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor="auditor-model" className="text-[11px] text-zinc-300">
          Audit model
        </label>
        <select
          id="auditor-model"
          value={value}
          aria-describedby={error ? 'auditor-model-hint auditor-model-error' : 'auditor-model-hint'}
          aria-invalid={error ? true : undefined}
          onChange={(e) => onChange(e.target.value as AuditorModel)}
          className={`bg-zinc-950 border rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none ${error ? 'border-rose-500/60' : 'border-zinc-800'}`}
        >
          {!isAuditorModel(value) && <option value={value}>{auditorModelLabel(value)}</option>}
          {AUDITOR_MODELS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
      <p id="auditor-model-hint" className="mt-1 text-[11px] text-zinc-500">
        {MODEL_HINT}
      </p>
      {error && (
        <p id="auditor-model-error" role="alert" className="mt-1 text-[11px] text-rose-400" data-testid="auditor-model-error">
          {error}
        </p>
      )}
    </div>
  );
}
