import re

def fix_file(filepath):
    with open(filepath, 'r') as f:
        text = f.read()

    # Find update_report_job calls
    # We will search for 'store.update_report_job(' and find its matching closing parenthesis.
    
    idx = 0
    new_text = ""
    while True:
        pos = text.find('store.update_report_job(', idx)
        if pos == -1:
            new_text += text[idx:]
            break
        new_text += text[idx:pos]
        
        # find matching paren
        paren_count = 0
        in_string = False
        escape = False
        end_pos = pos + len('store.update_report_job(')
        for i in range(end_pos, len(text)):
            c = text[i]
            if escape:
                escape = False
                continue
            if c == '\\':
                escape = True
            elif c == '"':
                in_string = not in_string
            elif not in_string:
                if c == '(':
                    paren_count += 1
                elif c == ')':
                    if paren_count == 0:
                        end_pos = i
                        break
                    paren_count -= 1
        
        args_str = text[pos + len('store.update_report_job('):end_pos]
        # split args correctly
        args = []
        current_arg = ""
        paren_count = 0
        in_string = False
        escape = False
        for c in args_str:
            if escape:
                escape = False
                current_arg += c
                continue
            if c == '\\':
                escape = True
                current_arg += c
            elif c == '"':
                in_string = not in_string
                current_arg += c
            elif not in_string:
                if c == '(':
                    paren_count += 1
                    current_arg += c
                elif c == ')':
                    paren_count -= 1
                    current_arg += c
                elif c == ',' and paren_count == 0:
                    args.append(current_arg)
                    current_arg = ""
                else:
                    current_arg += c
            else:
                current_arg += c
        args.append(current_arg)
        
        # We know it used to take 9 args: id, state, stage, sections_done, elements_done, tool_calls_done, current_tool, warning, error
        # Now it takes 7 args: id, state, stage, sections_done, elements_done, warning, error
        if len(args) == 9:
            args = args[:5] + args[7:]
        elif len(args) == 8:
            args = args[:5] + args[6:]
        
        # Reconstruct
        new_text += 'store.update_report_job(' + ','.join(args) + ')'
        idx = end_pos + 1

    with open(filepath, 'w') as f:
        f.write(new_text)

fix_file('crates/argos-osint-core/src/intel_recon/jobs.rs')
fix_file('crates/argos-osint-core/src/intel_recon/worker.rs')

